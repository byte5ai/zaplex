use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use super::*;
use tempfile::tempdir;

#[test]
fn copy_temp_id_exhaustion_is_fallible_and_never_reuses_an_id() {
    let counter = AtomicU64::new(u64::MAX);

    let outcome = std::panic::catch_unwind(|| next_copy_temp_sequence(&counter));

    assert!(outcome.is_ok(), "copy temp ID exhaustion must not panic");
    assert!(outcome.unwrap().is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn tokenless_file_and_directory_never_authorize_identity_bound_cleanup() {
    let file = StableEntryIdentity {
        file_type: FileEntryType::File,
        size: 4,
        object_id: String::new(),
        revision: "same-metadata".to_string(),
    };
    let directory = StableEntryIdentity {
        file_type: FileEntryType::Directory,
        size: 0,
        object_id: String::new(),
        revision: "same-metadata".to_string(),
    };

    assert!(!has_immutable_object_token(&file));
    assert!(!has_immutable_object_token(&directory));
}

#[cfg(unix)]
#[test]
fn guarded_file_delete_preserves_identical_foreign_replacement() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source.bin"), b"same").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf()).with_before_guarded_delete({
        let root = root.path().to_path_buf();
        move |_| {
            fs::remove_file(root.join("source.bin")).unwrap();
            fs::write(root.join("source.bin"), b"same").unwrap();
        }
    });
    let identity = backend.stable_identity(Path::new("/source.bin")).unwrap();
    let digest = format!("{:x}", Sha256::digest(b"same"));

    backend
        .delete_file_if_matches(Path::new("/source.bin"), &identity, &digest)
        .expect_err("the anchored object was replaced before isolation");

    assert_eq!(fs::read(root.path().join("source.bin")).unwrap(), b"same");
}

#[cfg(unix)]
#[test]
fn guarded_directory_delete_preserves_empty_foreign_replacement() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf()).with_before_guarded_delete({
        let root = root.path().to_path_buf();
        move |_| {
            fs::remove_dir(root.join("source")).unwrap();
            fs::create_dir(root.join("source")).unwrap();
        }
    });
    let identity = backend.stable_identity(Path::new("/source")).unwrap();

    backend
        .delete_empty_dir_if_matches(Path::new("/source"), &identity)
        .expect_err("the anchored directory was replaced before isolation");

    assert!(root.path().join("source").is_dir());
}

#[test]
fn isolated_file_delete_failure_restores_original_path() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("source.bin"), b"source").unwrap();
    let backend =
        InMemorySftpBackend::new(root.path().to_path_buf()).with_isolated_delete_failure();
    let identity = backend.stable_identity(Path::new("/source.bin")).unwrap();
    let digest = format!("{:x}", Sha256::digest(b"source"));

    backend
        .delete_file_if_matches(Path::new("/source.bin"), &identity, &digest)
        .expect_err("the injected tombstone delete must fail");

    assert_eq!(fs::read(root.path().join("source.bin")).unwrap(), b"source");
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-delete")
    }));
}

#[test]
fn isolated_directory_delete_failure_restores_original_path() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    let backend =
        InMemorySftpBackend::new(root.path().to_path_buf()).with_isolated_delete_failure();
    let identity = backend.stable_identity(Path::new("/source")).unwrap();

    backend
        .delete_empty_dir_if_matches(Path::new("/source"), &identity)
        .expect_err("the injected directory tombstone delete must fail");

    assert!(root.path().join("source").is_dir());
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("zaplex-delete")
    }));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review10_preflight_rename_swap_window_retains_and_reports_foreign_source() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_preflight_source_replacement_before_rename();

    let error = backend
        .preflight_safe_mutation(Path::new("/target.bin"), false)
        .expect_err("a swapped probe source must fail the capability preflight");
    let replacement = backend
        .preflight_mutation_replacement()
        .expect("the rename-window replacement hook must run");
    let foreign = fs::read_dir(root.path())
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| fs::read(entry.path()).is_ok_and(|bytes| bytes == b"foreign-rename"))
        .expect("the foreign rename candidate must remain reachable");
    let remote = backend.to_remote(&foreign.path());

    assert!(
        error.recovery_paths().contains(&remote),
        "the foreign candidate must remain reachable in recovery: {error:?}"
    );
    assert_eq!(
        foreign.path(),
        replacement,
        "preflight must reject the swapped source before mutating it"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review10_preflight_exchange_swap_window_retains_and_reports_foreign_source() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_preflight_source_replacement_before_exchange();

    let error = backend
        .preflight_safe_mutation(Path::new("/target.bin"), true)
        .expect_err("a swapped exchange source must fail the capability preflight");
    let replacement = backend
        .preflight_mutation_replacement()
        .expect("the exchange-window replacement hook must run");
    assert_eq!(
        fs::read(&replacement).unwrap(),
        b"foreign-exchange",
        "preflight must reject the swapped source before exchanging it"
    );
    let foreign_remote = backend.to_remote(&replacement);

    assert!(
        error.recovery_paths().contains(&foreign_remote),
        "every possible foreign candidate must remain reachable in recovery: {error:?}"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review11_first_noreplace_probe_revalidates_both_anchors_before_mutation() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_preflight_source_replacements_before_reject()
        .with_ignored_noreplace_probe_semantics();

    let error = backend
        .preflight_safe_mutation(Path::new("/target.bin"), false)
        .expect_err("replaced negative-probe operands must fail before mutation");
    let replacements = backend.preflight_reject_replacements();

    assert_eq!(replacements.len(), 2);
    assert_eq!(fs::read(&replacements[0]).unwrap(), b"foreign-first");
    assert_eq!(fs::read(&replacements[1]).unwrap(), b"foreign-second");
    for replacement in replacements {
        assert!(
            error
                .recovery_paths()
                .contains(&backend.to_remote(&replacement)),
            "every foreign probe operand must remain reachable: {error:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn review12_directory_create_never_anchors_a_visible_replacement() {
    let root = tempdir().unwrap();
    let retained = root.path().join("review12-original-directory");
    let hook_called = Arc::new(AtomicBool::new(false));
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_directory_create_before_anchor({
            let retained = retained.clone();
            let hook_called = hook_called.clone();
            move |visible| {
                hook_called.store(true, Ordering::SeqCst);
                fs::rename(visible, &retained).unwrap();
                fs::create_dir(visible).unwrap();
                fs::write(visible.join("foreign.bin"), b"foreign").unwrap();
            }
        });
    let visible = Path::new("/stage");

    let result = backend.create_dir_with_ownership_anchor(visible);

    assert!(
        result.is_err(),
        "the protected reservation must fail closed when the create/open seam is attacked"
    );
    assert!(
        hook_called.load(Ordering::SeqCst),
        "the adversarial seam must run exactly after create and before anchor acquisition"
    );
    assert!(retained.is_dir());
    assert!(!root.path().join("stage").exists());
}

#[cfg(unix)]
#[test]
fn review13_directory_reservation_never_anchors_a_private_path_replacement() {
    let root = tempdir().unwrap();
    let replacement = root.path().join("review13-private-replacement");
    let hook_called = Arc::new(AtomicBool::new(false));
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_directory_create_before_anchor({
            let replacement = replacement.clone();
            let hook_called = hook_called.clone();
            move |private| {
                hook_called.store(true, Ordering::SeqCst);
                fs::rename(private, &replacement).unwrap();
                fs::create_dir(private).unwrap();
                fs::write(private.join("foreign.bin"), b"foreign").unwrap();
            }
        });

    let result = backend.create_dir_with_ownership_anchor(Path::new("/stage"));

    assert!(
        result.is_err(),
        "the protected reservation must reject a private-path replacement"
    );
    assert!(
        hook_called.load(Ordering::SeqCst),
        "the test must exercise the exact create-to-anchor window"
    );
    assert!(replacement.is_dir());
    assert!(!root.path().join("stage").exists());
}

#[test]
fn review13_every_post_create_directory_failure_reports_the_owned_artifact() {
    for failure in [
        DirectoryReservationFailure::Open,
        DirectoryReservationFailure::Identity,
        DirectoryReservationFailure::Match,
        DirectoryReservationFailure::Publish,
    ] {
        let root = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_directory_reservation_failure(failure);

        let error = match backend.create_dir_with_ownership_anchor(Path::new("/stage")) {
            Ok(_) => panic!("the requested reservation step must fail"),
            Err(error) => error,
        };

        assert!(
            !error.recovery_paths().is_empty()
                || fs::read_dir(root.path()).unwrap().next().is_none(),
            "{failure:?} must either clean the owned reservation or expose it for recovery: {error:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn review14_directory_namespace_stays_on_the_backend_filesystem() {
    use std::os::unix::fs::MetadataExt;

    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    fs::create_dir(&root).unwrap();
    let backend = InMemorySftpBackend::new(root.clone());

    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap()
        .expect("local directory reservations must retain an anchor");

    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .expect("the reservation namespace must be initialized");
    assert_eq!(
        fs::metadata(&namespace).unwrap().dev(),
        fs::metadata(&root).unwrap().dev(),
        "the reservation namespace must stay on the destination filesystem"
    );
}

#[cfg(unix)]
#[test]
fn review14_directory_namespace_replacement_is_rejected_before_child_create() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/first"))
        .unwrap()
        .expect("the initial directory reservation must succeed");
    fs::remove_dir(root.path().join("first")).unwrap();

    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .expect("the reservation namespace must be initialized");
    let retained = namespace.with_extension("retained");
    fs::rename(&namespace, &retained).unwrap();
    fs::create_dir(&namespace).unwrap();
    fs::write(namespace.join("foreign.bin"), b"foreign").unwrap();

    let error = match backend.create_dir_with_ownership_anchor(Path::new("/second")) {
        Ok(_) => panic!("a replaced namespace must be rejected before creating a child"),
        Err(error) => error,
    };
    assert!(!error.to_string().is_empty());
    assert_eq!(fs::read(namespace.join("foreign.bin")).unwrap(), b"foreign");
    assert!(!root.path().join("second").exists());
}

#[cfg(unix)]
#[test]
fn review14_backend_paths_cannot_escape_root_or_enter_reservation_namespace() {
    use std::os::unix::fs::symlink;

    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    let outside = outer.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.bin"), b"secret").unwrap();
    symlink(&outside, root.join("escape")).unwrap();
    let backend = InMemorySftpBackend::new(root.clone());

    assert!(
        backend.lstat(Path::new("/../outside/secret.bin")).is_err(),
        "parent components must never escape the backend root"
    );
    assert!(
        backend.lstat(Path::new("/escape/secret.bin")).is_err(),
        "symlink traversal must never escape the backend root"
    );
    assert!(
        backend.stat(Path::new("/escape")).is_err(),
        "a final symlink target must never escape the backend root"
    );

    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap()
        .expect("the protected reservation must be created");
    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .expect("the reservation namespace must be initialized");
    let remote_namespace = backend.to_remote(&namespace);
    assert!(
        backend.list_dir(&remote_namespace).is_err(),
        "the private reservation namespace must not be remotely addressable"
    );
}

#[cfg(unix)]
#[test]
fn review14_restart_rediscovers_owned_reservation_without_adopting_foreign_child() {
    let root = tempdir().unwrap();
    let (namespace, owned_local) = {
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_directory_reservation_failure(DirectoryReservationFailure::Identity);
        let error = match backend.create_dir_with_ownership_anchor(Path::new("/stage")) {
            Ok(_) => panic!("the injected post-create failure must retain an owned reservation"),
            Err(error) => error,
        };
        let namespace = backend
            .directory_reservation_namespace_path_for_test()
            .expect("the reservation namespace must exist");
        let recovery_path = error
            .recovery_paths()
            .first()
            .expect("the retained reservation must be reported");
        let owned_local = backend.to_local(recovery_path).unwrap();
        fs::create_dir(namespace.join("foreign-child")).unwrap();
        fs::write(namespace.join("foreign-child/foreign.bin"), b"foreign").unwrap();
        (namespace, owned_local)
    };

    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let recovered = backend.startup_recovery_paths_for_test();
    assert_eq!(
        recovered.len(),
        1,
        "restart must rediscover exactly the owned reservation"
    );
    let recovery_path = &recovered[0];
    let identity = backend
        .cleanup_recovery_identity(recovery_path)
        .expect("the rediscovered reservation must retain its cleanup identity");
    let anchor = backend
        .cleanup_recovery_anchor(recovery_path)
        .expect("the rediscovered reservation must retain a live anchor");
    assert!(anchor.matches_path(recovery_path).unwrap());
    backend
        .delete_empty_dir_if_matches(recovery_path, &identity)
        .expect("the rediscovered owned reservation must be retryable");
    backend
        .release_cleanup_recovery_path(recovery_path)
        .unwrap();

    assert!(!owned_local.exists());
    assert_eq!(
        fs::read(namespace.join("foreign-child/foreign.bin")).unwrap(),
        b"foreign"
    );
}

#[cfg(unix)]
#[test]
fn review15_forged_namespace_and_child_markers_are_never_adopted() {
    use std::os::unix::fs::PermissionsExt;

    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    fs::create_dir(&root).unwrap();
    let probe = InMemorySftpBackend::new(root.clone());
    let namespace = root
        .parent()
        .unwrap()
        .join(probe.directory_reservation_namespace_name());
    drop(probe);
    fs::create_dir(&namespace).unwrap();
    fs::set_permissions(&namespace, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        namespace.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER),
        b"zaplex-directory-reservations-v1\n",
    )
    .unwrap();
    fs::set_permissions(
        namespace.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let foreign = namespace.join("forged-reservation");
    fs::create_dir(&foreign).unwrap();
    fs::write(
        namespace.join(format!(
            "forged-reservation{DIRECTORY_RESERVATION_MARKER_SUFFIX}"
        )),
        b"zaplex-owned-directory-v1\n",
    )
    .unwrap();

    let backend = InMemorySftpBackend::new(root);

    assert_eq!(
        backend.startup_recovery_paths_for_test(),
        Vec::<PathBuf>::new(),
        "public marker bytes and suffixes must not establish cleanup ownership"
    );
    assert!(foreign.is_dir(), "the forged child must remain untouched");
}

#[cfg(unix)]
#[test]
fn review15_namespace_symlink_and_tampered_child_marker_are_never_owned() {
    use std::os::unix::fs::symlink;

    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    let symlink_root = outer.path().join("symlink-root");
    let symlink_target = outer.path().join("foreign-namespace");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&symlink_root).unwrap();
    fs::create_dir(&symlink_target).unwrap();
    let probe = InMemorySftpBackend::new(symlink_root.clone());
    let candidate = symlink_root
        .parent()
        .unwrap()
        .join(probe.directory_reservation_namespace_name());
    drop(probe);
    symlink(&symlink_target, &candidate).unwrap();
    let restarted = InMemorySftpBackend::new(symlink_root);
    assert!(restarted.startup_recovery_paths_for_test().is_empty());
    assert!(symlink_target.is_dir());

    let backend = InMemorySftpBackend::new(root.clone())
        .with_directory_reservation_failure(DirectoryReservationFailure::Identity);
    assert!(backend
        .create_dir_with_ownership_anchor(Path::new("/retained"))
        .is_err());
    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .unwrap();
    let marker = fs::read_dir(&namespace)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(DIRECTORY_RESERVATION_MARKER_SUFFIX)
        })
        .unwrap()
        .path();
    fs::write(&marker, b"zaplex-owned-directory-v1\n").unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root);
    let recovered = restarted.startup_recovery_paths_for_test();
    assert_eq!(
        recovered.len(),
        2,
        "the private directory and forged marker remain visible but unresolved"
    );
    assert!(recovered
        .iter()
        .all(|path| restarted.cleanup_recovery_identity(path).is_none()));
}

#[cfg(unix)]
#[test]
fn review15_replacement_between_directory_create_and_anchor_is_never_published_or_deleted() {
    let root = tempdir().unwrap();
    let foreign_path = Arc::new(Mutex::new(None));
    let observed = foreign_path.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_directory_anchor_before_publish(move |private| {
            fs::remove_dir(private).unwrap();
            fs::create_dir(private).unwrap();
            fs::write(private.join("foreign.bin"), b"foreign").unwrap();
            *observed.lock().unwrap() = Some(private.to_path_buf());
        });

    let result = backend.create_dir_with_ownership_anchor(Path::new("/visible"));

    assert!(
        result.is_err(),
        "a replacement before anchor acquisition must fail closed"
    );
    let foreign = foreign_path.lock().unwrap().clone().unwrap();
    assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
    assert!(
        !root.path().join("visible").exists(),
        "the foreign replacement must never be published"
    );
}

#[cfg(unix)]
#[test]
fn review15_namespace_replacement_before_authentication_is_never_adopted() {
    let root = tempdir().unwrap();
    let replacement = Arc::new(Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_namespace_create_before_anchor(move |namespace| {
            fs::remove_dir(namespace).unwrap();
            fs::create_dir(namespace).unwrap();
            fs::write(namespace.join("foreign.bin"), b"foreign").unwrap();
            *observed.lock().unwrap() = Some(namespace.to_path_buf());
        });

    assert!(backend
        .create_dir_with_ownership_anchor(Path::new("/visible"))
        .is_err());
    let replacement = replacement.lock().unwrap().clone().unwrap();
    assert_eq!(
        fs::read(replacement.join("foreign.bin")).unwrap(),
        b"foreign"
    );
    assert!(!root.path().join("visible").exists());
    assert!(backend.startup_recovery_paths_for_test().is_empty());
}

#[cfg(unix)]
#[test]
fn review15_external_leaf_symlink_is_rejected_by_download_and_same_backend_copy() {
    use std::os::unix::fs::symlink;

    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    let outside = outer.path().join("outside-secret.bin");
    let download = outer.path().join("download.bin");
    fs::create_dir(&root).unwrap();
    fs::write(&outside, b"secret").unwrap();
    symlink(&outside, root.join("secret-link")).unwrap();
    let backend = InMemorySftpBackend::new(root.clone());

    assert!(
        backend
            .download_file(Path::new("/secret-link"), &download, None, None)
            .is_err(),
        "download must not follow a final file symlink outside the backend root"
    );
    assert!(
        backend
            .copy_file(Path::new("/secret-link"), Path::new("/copy.bin"))
            .is_err(),
        "same-backend copy must not follow a final file symlink outside the backend root"
    );
    assert!(!download.exists());
    assert!(!root.join("copy.bin").exists());
    assert_eq!(
        backend.lstat(Path::new("/secret-link")).unwrap().file_type,
        FileEntryType::Symlink,
        "lstat must keep the legitimate broken/final-symlink inspection contract"
    );
}

#[cfg(unix)]
#[test]
fn review15_successful_directory_publishes_leave_no_child_markers() {
    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    fs::create_dir(&root).unwrap();
    let backend = InMemorySftpBackend::new(root.clone());

    for index in 0..3 {
        let path = PathBuf::from(format!("/published-{index}"));
        backend
            .create_dir_with_ownership_anchor(&path)
            .unwrap()
            .expect("directory publish must retain its anchor");
        fs::remove_dir(root.join(format!("published-{index}"))).unwrap();
    }

    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .expect("the namespace must exist");
    let entries = fs::read_dir(namespace)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![DIRECTORY_RESERVATION_NAMESPACE_MARKER],
        "successful publishes must not accumulate per-reservation marker files"
    );
}

#[cfg(unix)]
#[test]
fn review15_marker_cleanup_failure_remains_retryable_after_restart() {
    let outer = tempdir().unwrap();
    let root = outer.path().join("backend-root");
    fs::create_dir(&root).unwrap();
    let backend = InMemorySftpBackend::new(root.clone()).with_directory_marker_cleanup_failure();
    let error = match backend.create_dir_with_ownership_anchor(Path::new("/published")) {
        Ok(_) => panic!("the injected marker cleanup must be reported"),
        Err(error) => error,
    };
    assert!(error.destination_committed());
    assert_eq!(error.recovery_paths().len(), 1);
    assert!(backend
        .cleanup_recovery_identity(&error.recovery_paths()[0])
        .is_some());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root);
    let recovered = restarted.startup_recovery_paths_for_test();
    assert_eq!(recovered.len(), 1);
    assert!(
        restarted.cleanup_recovery_identity(&recovered[0]).is_some(),
        "an authenticated retained marker must remain identity-bound and retryable"
    );
}

#[cfg(unix)]
#[test]
fn review16_namespace_create_to_anchor_swap_is_never_adopted() {
    let root = tempdir().unwrap();
    let replacement = Arc::new(std::sync::Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_namespace_create_before_anchor(move |namespace| {
            let retained = namespace.with_extension("review16-owned");
            fs::rename(namespace, &retained).unwrap();
            fs::create_dir(namespace).unwrap();
            fs::write(namespace.join("foreign.bin"), b"foreign").unwrap();
            *observed.lock().unwrap() = Some((namespace.to_path_buf(), retained));
        });

    let result = backend.create_dir_with_ownership_anchor(Path::new("/visible"));

    assert!(
        result.is_err(),
        "a directory substituted before anchor acquisition must never be authenticated"
    );
    let (foreign, retained) = replacement.lock().unwrap().clone().unwrap();
    assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
    assert!(retained.is_dir());
    assert!(!root.path().join("visible").exists());
}

#[cfg(unix)]
#[test]
fn review16_private_directory_create_to_anchor_swap_is_never_published() {
    let root = tempdir().unwrap();
    let replacement = Arc::new(std::sync::Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_directory_create_before_anchor(move |private| {
            let retained = private.with_extension("review16-owned");
            fs::rename(private, &retained).unwrap();
            fs::create_dir(private).unwrap();
            fs::write(private.join("foreign.bin"), b"foreign").unwrap();
            *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
        });

    let result = backend.create_dir_with_ownership_anchor(Path::new("/visible"));

    assert!(
        result.is_err(),
        "a private reservation substituted before anchor acquisition must never be published"
    );
    let (foreign, retained) = replacement.lock().unwrap().clone().unwrap();
    assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
    assert!(retained.is_dir());
    assert!(!root.path().join("visible").exists());
}

#[cfg(unix)]
#[test]
fn review16_writer_parent_swap_cannot_escape_backend_root() {
    use std::os::unix::fs::symlink;

    let outer = tempdir().unwrap();
    let root = outer.path().join("root");
    let outside = outer.path().join("outside");
    let retained = outer.path().join("retained-parent");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("parent")).unwrap();
    fs::create_dir(&outside).unwrap();
    let outside_for_hook = outside.clone();
    let backend = InMemorySftpBackend::new(root.clone()).with_after_writer_validation_before_open(
        move |local| {
            let parent = local.parent().unwrap();
            fs::rename(parent, &retained).unwrap();
            symlink(&outside_for_hook, parent).unwrap();
        },
    );

    assert!(
        backend
            .create_file_writer(Path::new("/parent/stage.bin"))
            .is_err(),
        "writer creation must resolve beneath an anchored root without following swapped parents"
    );
    assert!(
        !outside.join("stage.bin").exists(),
        "the swapped parent must never receive the transfer stage"
    );
}

#[test]
fn review16_file_stage_survives_restart_as_visible_recovery() {
    let root = tempdir().unwrap();
    let stage = Path::new("/.target.zaplex-transfer-crash-cutpoint");
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let mut writer = backend.create_file_writer(stage).unwrap();
    writer.write_chunk(b"partial-stage").unwrap();
    writer.flush().unwrap();
    drop(writer);
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());

    assert!(
        !restarted.startup_recovery_paths_for_test().is_empty(),
        "a file stage must be persisted before streaming and rediscovered after restart"
    );
}

#[test]
fn review16_backup_and_quarantine_cutpoints_survive_restart() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    for path in [
        Path::new("/.target.zaplex-backup-crash-cutpoint"),
        Path::new("/.source.zaplex-source-crash-cutpoint"),
    ] {
        let mut writer = backend.create_file_writer(path).unwrap();
        writer.write_chunk(b"retained").unwrap();
        writer.flush().unwrap();
    }
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());

    assert_eq!(
        restarted.startup_recovery_paths_for_test().len(),
        2,
        "backup and quarantine artifacts must remain independently retryable"
    );
}

#[cfg(unix)]
#[test]
fn review16_corrupt_registry_record_becomes_stable_unresolved_recovery() {
    use std::os::unix::fs::MetadataExt;

    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/published"))
        .unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let device = fs::metadata(root.path()).unwrap().dev();
    fs::write(registry.record_path(device), b"corrupt").unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());

    assert!(
        !restarted.startup_recovery_paths_for_test().is_empty(),
        "authenticated-registry read errors must become stable unresolved global recovery"
    );
}

#[test]
fn review16_populated_directory_stage_survives_restart_without_path_rebinding() {
    let root = tempdir().unwrap();
    let stage = Path::new("/.target.zaplex-tree-crash-cutpoint");
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(stage)
        .unwrap()
        .expect("directory stage must retain its creation anchor");
    let mut child = backend
        .create_file_writer(&stage.join("child.bin"))
        .unwrap();
    child.write_chunk(b"child").unwrap();
    child.flush().unwrap();
    drop(child);
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let recovered = restarted.startup_recovery_paths_for_test();

    assert_eq!(recovered, vec![stage.to_path_buf()]);
    assert!(
        restarted.cleanup_recovery_anchor(stage).is_some(),
        "restart must re-anchor the authenticated directory object, not recapture a path occupant"
    );
    assert_eq!(
        fs::read(
            root.path()
                .join(".target.zaplex-tree-crash-cutpoint/child.bin")
        )
        .unwrap(),
        b"child"
    );
}

#[cfg(unix)]
#[test]
fn review16_parallel_registry_start_and_record_updates_lose_no_artifact() {
    use std::sync::Barrier;
    use std::thread;

    let root = tempdir().unwrap();
    let start = Arc::new(Barrier::new(3));
    let workers = (0..2)
        .map(|index| {
            let root = root.path().to_path_buf();
            let start = start.clone();
            thread::spawn(move || {
                start.wait();
                let backend = InMemorySftpBackend::new(root);
                let path = PathBuf::from(format!("/.target-{index}.zaplex-transfer-parallel"));
                let mut writer = backend.create_file_writer(&path).unwrap();
                writer
                    .write_chunk(format!("worker-{index}").as_bytes())
                    .unwrap();
                writer.flush().unwrap();
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    for worker in workers {
        worker.join().unwrap();
    }

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let mut recovered = restarted.startup_recovery_paths_for_test();
    recovered.sort();

    assert_eq!(recovered.len(), 2);
    assert!(recovered
        .iter()
        .all(|path| restarted.cleanup_recovery_anchor(path).is_some()));
}

#[cfg(unix)]
#[test]
fn review17_empty_directory_replacement_is_never_deleted_after_anchor_failure() {
    let root = tempdir().unwrap();
    let replacement = Arc::new(std::sync::Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_directory_create_before_anchor(move |private| {
            let retained = private.with_extension("review17-owned");
            fs::rename(private, &retained).unwrap();
            fs::create_dir(private).unwrap();
            *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
        });

    assert!(
        backend
            .create_dir_with_ownership_anchor(Path::new("/visible"))
            .is_err(),
        "a replaced private directory must fail closed"
    );

    let (foreign, retained) = replacement.lock().unwrap().clone().unwrap();
    assert!(
        foreign.is_dir(),
        "the empty foreign replacement was deleted"
    );
    assert!(
        retained.is_dir(),
        "the original reservation must be retained"
    );
    assert!(
        backend.startup_recovery_paths_for_test().len() >= 2,
        "the original and replacement candidates must remain separately visible"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review17_exchange_crash_retains_identity_bound_displaced_target_record() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("target.bin"), b"target").unwrap();
    let stage = Path::new("/.target.bin.zaplex-transfer-review17");
    let target = Path::new("/target.bin");
    let backend = InMemorySftpBackend::new(root.path().to_path_buf()).with_after_replace(|_| {
        panic!("review17 crash cutpoint immediately after exchange");
    });
    let mut writer = backend.create_file_writer(stage).unwrap();
    writer.write_chunk(b"stage").unwrap();
    writer.flush().unwrap();
    drop(writer);

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.replace(stage, target)
    }));
    assert!(crashed.is_err());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(fs::read(root.path().join("target.bin")).unwrap(), b"stage");
    assert_eq!(
        fs::read(root.path().join(".target.bin.zaplex-transfer-review17")).unwrap(),
        b"target"
    );
    let identity = restarted
        .cleanup_recovery_identity(stage)
        .expect("the displaced target must retain an identity-bound persistent recovery record");
    assert!(
        restarted.cleanup_recovery_anchor(stage).is_some(),
        "restart must retain a live cleanup anchor for the displaced target"
    );
    let digest = format!("{:x}", Sha256::digest(b"target"));
    restarted
        .delete_file_if_matches(stage, &identity, &digest)
        .expect("identity-bound retry must clean the displaced target");
    assert!(!root
        .path()
        .join(".target.bin.zaplex-transfer-review17")
        .exists());
    drop(restarted);

    let resolved = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(
        !resolved
            .startup_recovery_paths_for_test()
            .contains(&stage.to_path_buf()),
        "terminal cleanup must remove the persistent exchange record"
    );
}

#[cfg(unix)]
#[test]
fn review17_missing_authenticated_namespace_is_stable_unresolved_recovery() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/visible"))
        .unwrap();
    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .unwrap();
    drop(backend);
    fs::remove_dir_all(&namespace).unwrap();

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let first = restarted.startup_recovery_paths_for_test();
    let second = restarted.startup_recovery_paths_for_test();

    assert_eq!(first, second);
    assert_eq!(
        first.len(),
        1,
        "a missing authenticated namespace must remain globally visible"
    );
}

#[cfg(unix)]
#[test]
fn review17_namespace_read_dir_failure_is_stable_unresolved_recovery() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/visible"))
        .unwrap();

    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::ReadDirectory);
    let first = backend.startup_recovery_paths_for_test();
    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::ReadDirectory);
    let second = backend.startup_recovery_paths_for_test();

    assert_eq!(first, second);
    assert_eq!(
        first.len(),
        1,
        "a namespace read_dir failure must become one bounded unresolved activity"
    );
}

#[cfg(unix)]
#[test]
fn review17_namespace_entry_failure_is_stable_unresolved_recovery() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/visible"))
        .unwrap();

    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::DirectoryEntry);
    let first = backend.startup_recovery_paths_for_test();
    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::DirectoryEntry);
    let second = backend.startup_recovery_paths_for_test();

    assert_eq!(first, second);
    assert_eq!(
        first.len(),
        1,
        "an iterator error must become one bounded unresolved activity"
    );
}

#[cfg(unix)]
#[test]
fn review18_directory_create_anchor_failure_survives_two_restarts_with_both_candidates() {
    for populated in [false, true] {
        let root = tempdir().unwrap();
        let candidates = Arc::new(std::sync::Mutex::new(None));
        let observed = candidates.clone();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_after_directory_create_before_anchor(move |private| {
                let retained = private.with_extension("review18-owned");
                fs::rename(private, &retained).unwrap();
                fs::create_dir(private).unwrap();
                if populated {
                    fs::write(private.join("foreign.bin"), b"foreign").unwrap();
                }
                *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
            });

        assert!(
            backend
                .create_dir_with_ownership_anchor(Path::new("/visible"))
                .is_err(),
            "the swapped reservation must fail closed"
        );
        let (foreign, retained) = candidates.lock().unwrap().clone().unwrap();
        drop(backend);

        let first_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        let first = first_restart.startup_recovery_paths_for_test();
        assert!(
            first.len() >= 2,
            "both create-to-anchor candidates must survive restart"
        );
        assert!(foreign.is_dir());
        assert!(retained.is_dir());
        drop(first_restart);

        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(
            second_restart.startup_recovery_paths_for_test(),
            first,
            "candidate recovery must remain bounded and idempotent"
        );
        assert!(foreign.is_dir());
        assert!(retained.is_dir());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review18_failed_final_exchange_check_restores_the_stage_record_across_restarts() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("target.bin"), b"target").unwrap();
    let stage = Path::new("/.target.bin.zaplex-transfer-review18");
    let target = Path::new("/target.bin");
    let retained = root.path().join("retained-target.bin");
    let backend = InMemorySftpBackend::new(root.path().to_path_buf()).with_before_replace({
        let target = root.path().join("target.bin");
        let retained = retained.clone();
        move |_| {
            fs::rename(&target, &retained).unwrap();
            fs::write(&target, b"foreign").unwrap();
        }
    });
    let mut writer = backend.create_file_writer(stage).unwrap();
    writer.write_chunk(b"stage").unwrap();
    writer.flush().unwrap();
    drop(writer);

    backend
        .replace(stage, target)
        .expect_err("a target swap before exchange must fail closed");
    drop(backend);

    let first_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        fs::read(root.path().join(".target.bin.zaplex-transfer-review18")).unwrap(),
        b"stage"
    );
    assert!(
        first_restart.cleanup_recovery_anchor(stage).is_some(),
        "the original stage record must be restored after a proven non-apply"
    );
    let first = first_restart.startup_recovery_paths_for_test();
    drop(first_restart);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    assert!(
        second_restart.cleanup_recovery_anchor(stage).is_some(),
        "the stage must remain identity-bound after the second restart"
    );
    assert_eq!(fs::read(retained).unwrap(), b"target");
    assert_eq!(
        fs::read(root.path().join("target.bin")).unwrap(),
        b"foreign"
    );
}

#[cfg(unix)]
#[test]
fn review18_concrete_marker_file_type_error_survives_restart_with_its_path() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_directory_reservation_failure(DirectoryReservationFailure::Identity);
    assert!(
        backend
            .create_dir_with_ownership_anchor(Path::new("/visible"))
            .is_err(),
        "the test must retain a private reservation and marker"
    );
    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .unwrap();
    let marker = fs::read_dir(&namespace)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .ends_with(DIRECTORY_RESERVATION_MARKER_SUFFIX)
            })
        })
        .unwrap();
    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::MarkerFileType);
    let expected = InMemorySftpBackend::unresolved_registry_path(
        "namespace-marker-file-type",
        &marker.display().to_string(),
    );
    assert!(backend
        .startup_recovery_paths_for_test()
        .contains(&expected));
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(
        restarted
            .startup_recovery_paths_for_test()
            .contains(&expected),
        "the concrete marker failure must persist across startup"
    );
    let first = restarted.startup_recovery_paths_for_test();
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
}

#[cfg(unix)]
#[test]
fn review18_concrete_second_pass_entry_error_survives_restart_with_its_path() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    backend
        .create_dir_with_ownership_anchor(Path::new("/visible"))
        .unwrap();
    let namespace = backend
        .directory_reservation_namespace_path_for_test()
        .unwrap();
    let entry = namespace.join("concrete-unclaimed-entry");
    fs::write(&entry, b"foreign").unwrap();

    backend.rescan_namespace_with_failure_for_test(NamespaceScanFailure::UnclaimedFileType);
    let expected = InMemorySftpBackend::unresolved_registry_path(
        "namespace-unclaimed-file-type",
        &entry.display().to_string(),
    );
    assert!(backend
        .startup_recovery_paths_for_test()
        .contains(&expected));
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&expected));
    let first = restarted.startup_recovery_paths_for_test();
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    assert_eq!(fs::read(entry).unwrap(), b"foreign");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review18_file_restore_interference_preserves_every_visible_candidate_after_restart() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let retained = root.path().join("retained-source.bin");
    fs::write(&source, b"source").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_guarded_rename_check_before_mutation({
            let source = source.clone();
            let retained = retained.clone();
            move |_, _| {
                fs::rename(&source, &retained).unwrap();
                fs::write(&source, b"foreign").unwrap();
            }
        })
        .with_before_guarded_rename_restore(|source, _| {
            fs::remove_file(source).unwrap();
            fs::write(source, b"interference").unwrap();
        });
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/source.bin"))
        .unwrap()
        .unwrap();

    backend
        .rename_if_matches(
            Path::new("/source.bin"),
            Path::new("/.source.bin.zaplex-source-review18"),
            anchor,
        )
        .expect_err("restore-window interference must remain unresolved");

    assert_eq!(fs::read(&source).unwrap(), b"interference");
    assert_eq!(fs::read(&retained).unwrap(), b"source");
    assert_eq!(
        fs::read(root.path().join(".source.bin.zaplex-source-review18")).unwrap(),
        b"foreign"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted.startup_recovery_paths_for_test().contains(
        &InMemorySftpBackend::unresolved_registry_path("guarded-isolation-source", "/source.bin")
    ));
    assert!(restarted.startup_recovery_paths_for_test().contains(
        &InMemorySftpBackend::unresolved_registry_path(
            "guarded-isolation-quarantine",
            "/.source.bin.zaplex-source-review18"
        )
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review18_directory_restore_interference_preserves_every_visible_candidate_after_restart() {
    let root = tempdir().unwrap();
    let source = root.path().join("source");
    let retained = root.path().join("retained-source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("value.bin"), b"source").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_guarded_rename_check_before_mutation({
            let source = source.clone();
            let retained = retained.clone();
            move |_, _| {
                fs::rename(&source, &retained).unwrap();
                fs::create_dir(&source).unwrap();
                fs::write(source.join("value.bin"), b"foreign").unwrap();
            }
        })
        .with_before_guarded_rename_restore(|source, _| {
            fs::remove_dir(source).unwrap();
            fs::create_dir(source).unwrap();
            fs::write(source.join("value.bin"), b"interference").unwrap();
        });
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/source"))
        .unwrap()
        .unwrap();

    backend
        .rename_if_matches(
            Path::new("/source"),
            Path::new("/.source.zaplex-source-review18"),
            anchor,
        )
        .expect_err("directory restore-window interference must remain unresolved");

    assert_eq!(fs::read(source.join("value.bin")).unwrap(), b"interference");
    assert_eq!(fs::read(retained.join("value.bin")).unwrap(), b"source");
    assert_eq!(
        fs::read(root.path().join(".source.zaplex-source-review18/value.bin")).unwrap(),
        b"foreign"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted.startup_recovery_paths_for_test().contains(
        &InMemorySftpBackend::unresolved_registry_path("guarded-isolation-source", "/source")
    ));
    assert!(restarted.startup_recovery_paths_for_test().contains(
        &InMemorySftpBackend::unresolved_registry_path(
            "guarded-isolation-quarantine",
            "/.source.zaplex-source-review18"
        )
    ));
}

#[cfg(unix)]
fn assert_review18_placeholder_replacement_survives_restart(
    directory: bool,
    after_isolation: bool,
) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/placeholder")
    } else {
        Path::new("/placeholder.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"placeholder").unwrap();
    }
    let observation = Arc::new(std::sync::Mutex::new(None));
    let observed = observation.clone();
    let replacement = move |mutated: &Path| {
        let retained = mutated.with_file_name(format!(
            ".{}.zaplex-delete-placeholder-retained",
            mutated
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
        ));
        fs::rename(mutated, &retained).unwrap();
        if directory {
            fs::create_dir(mutated).unwrap();
            fs::write(mutated.join("foreign.bin"), b"foreign").unwrap();
        } else {
            fs::write(mutated, b"foreign").unwrap();
        }
        *observed.lock().unwrap() = Some((mutated.to_path_buf(), retained));
    };
    let backend = if after_isolation {
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_before_placeholder_tombstone_cleanup(replacement)
    } else {
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_before_placeholder_isolation(replacement)
    };
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .cleanup_isolation_placeholder(path, anchor, &identity)
        .expect_err("a replaced placeholder must remain identity-bound recovery");

    let (foreign, retained) = observation.lock().unwrap().clone().unwrap();
    if directory {
        assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
        assert!(retained.is_dir());
    } else {
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read(&retained).unwrap(), b"placeholder");
    }
    let retained_backend_path =
        PathBuf::from("/").join(retained.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(
        restarted
            .startup_recovery_paths_for_test()
            .contains(&retained_backend_path),
        "the anchored placeholder candidate must survive restart"
    );
    assert!(
        restarted
            .cleanup_recovery_anchor(&retained_backend_path)
            .is_some(),
        "the retained placeholder must stay identity-bound"
    );
    let first = restarted.startup_recovery_paths_for_test();
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    if directory {
        assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
    } else {
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[cfg(unix)]
#[test]
fn review18_file_placeholder_swap_before_isolation_is_foreign_safe_and_retryable() {
    assert_review18_placeholder_replacement_survives_restart(false, false);
}

#[cfg(unix)]
#[test]
fn review18_directory_placeholder_swap_before_isolation_is_foreign_safe_and_retryable() {
    assert_review18_placeholder_replacement_survives_restart(true, false);
}

#[cfg(unix)]
#[test]
fn review18_file_placeholder_swap_after_isolation_is_foreign_safe_and_retryable() {
    assert_review18_placeholder_replacement_survives_restart(false, true);
}

#[cfg(unix)]
#[test]
fn review18_directory_placeholder_swap_after_isolation_is_foreign_safe_and_retryable() {
    assert_review18_placeholder_replacement_survives_restart(true, true);
}

#[cfg(unix)]
fn assert_review19_final_placeholder_delete_is_foreign_safe(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/placeholder")
    } else {
        Path::new("/placeholder.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"placeholder").unwrap();
    }
    let observation = Arc::new(std::sync::Mutex::new(None));
    let observed = observation.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_placeholder_final_check_before_delete(move |tombstone| {
            let retained = tombstone.with_extension("review19-owned");
            fs::rename(tombstone, &retained).unwrap();
            if directory {
                fs::create_dir(tombstone).unwrap();
            } else {
                fs::write(tombstone, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some((tombstone.to_path_buf(), retained));
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .cleanup_isolation_placeholder(path, anchor, &identity)
        .expect_err("the final check-to-delete swap must remain recoverable");

    let (foreign, retained) = observation.lock().unwrap().clone().unwrap();
    if directory {
        assert!(
            foreign.is_dir(),
            "the empty foreign directory must not be deleted"
        );
        assert!(retained.is_dir());
    } else {
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read(&retained).unwrap(), b"placeholder");
    }
    let retained_backend_path =
        PathBuf::from("/").join(retained.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&retained_backend_path));
    assert!(restarted
        .cleanup_recovery_anchor(&retained_backend_path)
        .is_some());
    let first = restarted.startup_recovery_paths_for_test();
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[cfg(unix)]
#[test]
fn review19_file_placeholder_swap_after_final_check_never_deletes_foreign() {
    assert_review19_final_placeholder_delete_is_foreign_safe(false);
}

#[cfg(unix)]
#[test]
fn review19_directory_placeholder_swap_after_final_check_never_deletes_foreign() {
    assert_review19_final_placeholder_delete_is_foreign_safe(true);
}

#[cfg(unix)]
fn assert_review19_private_cleanup_parent_swap_is_foreign_safe(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/placeholder")
    } else {
        Path::new("/placeholder.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"placeholder").unwrap();
    }
    let foreign = Arc::new(std::sync::Mutex::new(None));
    let observed = foreign.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_before_private_placeholder_unlink(move |namespace, private| {
            let retained = namespace.with_extension("review19-retained");
            fs::rename(namespace, &retained).unwrap();
            fs::create_dir(namespace).unwrap();
            let replacement = namespace.join(private.file_name().unwrap());
            if directory {
                fs::create_dir(&replacement).unwrap();
            } else {
                fs::write(&replacement, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some(replacement);
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    let cleanup = backend.cleanup_isolation_placeholder(path, anchor, &identity);
    assert!(
        cleanup.is_err(),
        "a replaced private cleanup parent must remain visible recovery"
    );
    assert!(
        foreign.lock().unwrap().is_some(),
        "private cleanup hook was not reached: {cleanup:?}"
    );
    let foreign = foreign.lock().unwrap().clone().unwrap();
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
    }
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let first = restarted.startup_recovery_paths_for_test();
    assert!(!first.is_empty());
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[cfg(unix)]
#[test]
fn review19_file_private_cleanup_parent_swap_never_deletes_foreign() {
    assert_review19_private_cleanup_parent_swap_is_foreign_safe(false);
}

#[cfg(unix)]
#[test]
fn review19_directory_private_cleanup_parent_swap_never_deletes_foreign() {
    assert_review19_private_cleanup_parent_swap_is_foreign_safe(true);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_review19_exchange_candidate_discovery_survives_restart(directory: bool) {
    let root = tempdir().unwrap();
    let source_path = if directory {
        Path::new("/source")
    } else {
        Path::new("/source.bin")
    };
    let quarantine_path = if directory {
        Path::new("/.source.zaplex-source-review19")
    } else {
        Path::new("/.source.bin.zaplex-source-review19")
    };
    let source = root.path().join(source_path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&source).unwrap();
        fs::write(source.join("value.bin"), b"source").unwrap();
    } else {
        fs::write(&source, b"source").unwrap();
    }
    let retained = Arc::new(std::sync::Mutex::new(None));
    let observed = retained.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_guarded_exchange_before_classification(move |source, quarantine| {
            let retained_placeholder = source.with_extension("review19-placeholder");
            let retained_source = quarantine.with_extension("review19-source");
            fs::rename(source, &retained_placeholder).unwrap();
            fs::rename(quarantine, &retained_source).unwrap();
            if directory {
                fs::create_dir(source).unwrap();
                fs::create_dir(quarantine).unwrap();
            } else {
                fs::write(source, b"foreign-source").unwrap();
                fs::write(quarantine, b"foreign-quarantine").unwrap();
            }
            *observed.lock().unwrap() = Some((retained_placeholder, retained_source));
        });
    let anchor = backend
        .existing_entry_ownership_anchor(source_path)
        .unwrap()
        .unwrap();

    backend
        .rename_if_matches(source_path, quarantine_path, anchor)
        .expect_err("moving both exchange objects before classification must be unresolved");
    let (retained_placeholder, retained_source) = retained.lock().unwrap().clone().unwrap();
    let retained_placeholder_path =
        PathBuf::from("/").join(retained_placeholder.strip_prefix(root.path()).unwrap());
    let retained_source_path =
        PathBuf::from("/").join(retained_source.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let first = restarted.startup_recovery_paths_for_test();
    assert!(first.contains(&retained_placeholder_path));
    assert!(first.contains(&retained_source_path));
    assert!(restarted
        .cleanup_recovery_anchor(&retained_placeholder_path)
        .is_some());
    assert!(restarted
        .cleanup_recovery_anchor(&retained_source_path)
        .is_some());
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    if directory {
        assert_eq!(
            fs::read(retained_source.join("value.bin")).unwrap(),
            b"source"
        );
    } else {
        assert_eq!(fs::read(retained_source).unwrap(), b"source");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review19_file_exchange_discovers_both_moved_anchor_candidates() {
    assert_review19_exchange_candidate_discovery_survives_restart(false);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review19_directory_exchange_discovers_both_moved_anchor_candidates() {
    assert_review19_exchange_candidate_discovery_survives_restart(true);
}

#[cfg(unix)]
#[test]
fn review19_directory_create_candidates_retry_owned_without_mutating_replacement() {
    for populated in [false, true] {
        let root = tempdir().unwrap();
        let candidates = Arc::new(std::sync::Mutex::new(None));
        let observed = candidates.clone();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_after_directory_create_before_anchor(move |private| {
                let retained = private.with_extension("review19-owned");
                fs::rename(private, &retained).unwrap();
                fs::create_dir(private).unwrap();
                if populated {
                    fs::write(private.join("foreign.bin"), b"foreign").unwrap();
                }
                *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
            });
        assert!(
            backend
                .create_dir_with_ownership_anchor(Path::new("/visible"))
                .is_err(),
            "the create-to-anchor replacement must fail closed"
        );
        let (foreign, retained) = candidates.lock().unwrap().clone().unwrap();
        drop(backend);

        let restarted = Arc::new(InMemorySftpBackend::new(root.path().to_path_buf()));
        let recovery_path = restarted
            .startup_recovery_paths_for_test()
            .into_iter()
            .find(|path| restarted.cleanup_recovery_anchor(path).is_some())
            .expect("the owned physical candidate must have an identity-bound retry");
        let error = crate::sftp_manager::transfer_job::startup_backend_recovery_error(
            restarted.clone(),
            vec![recovery_path],
        );
        let recovery_id = error.recovery_id().expect("retry ID");
        crate::sftp_manager::transfer_job::retry_recovery(recovery_id).unwrap();

        assert!(!retained.exists(), "the owned candidate must be cleaned");
        assert!(foreign.is_dir(), "the replacement must remain");
        if populated {
            assert_eq!(fs::read(foreign.join("foreign.bin")).unwrap(), b"foreign");
        }
        drop(restarted);
        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        let remaining = second_restart.startup_recovery_paths_for_test();
        assert!(
            !remaining.is_empty()
                && remaining
                    .iter()
                    .all(|path| second_restart.cleanup_recovery_anchor(path).is_none()),
            "the foreign replacement must remain unresolved without cleanup authority"
        );
        drop(second_restart);
        let third_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(third_restart.startup_recovery_paths_for_test(), remaining);
    }
}

#[cfg(unix)]
#[test]
fn review19_sibling_discovery_failures_are_persistent_and_path_bound() {
    for failure in [
        SiblingRecoveryFailure::ReadDirectory,
        SiblingRecoveryFailure::DirectoryEntry,
        SiblingRecoveryFailure::AnchorProbe,
        SiblingRecoveryFailure::RegistryWrite,
    ] {
        let root = tempdir().unwrap();
        let local = root.path().join("owned.bin");
        fs::write(&local, b"owned").unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_sibling_recovery_failure(failure);
        let anchor = backend
            .existing_entry_ownership_anchor(Path::new("/owned.bin"))
            .unwrap()
            .unwrap();
        let identity = anchor.identity().unwrap();

        backend
            .persist_anchor_sibling_recovery(
                Path::new("/owned.bin"),
                anchor,
                &identity,
                "owned-isolation-placeholder",
            )
            .unwrap();
        drop(backend);

        let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
        let first = restarted.startup_recovery_paths_for_test();
        assert_eq!(
            first.len(),
            1,
            "{failure:?} must produce one bounded concrete unresolved activity"
        );
        assert_eq!(fs::read(&local).unwrap(), b"owned");
        drop(restarted);
        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
        assert_eq!(fs::read(local).unwrap(), b"owned");
    }
}

#[cfg(unix)]
#[test]
fn review19_hardlink_aliases_never_create_multiple_cleanup_authorizations() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    let alias = root.path().join("alias.bin");
    fs::write(&owned, b"owned").unwrap();
    fs::hard_link(&owned, &alias).unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    let diagnostics = backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-placeholder",
        )
        .unwrap();
    assert!(diagnostics.len() >= 2);
    assert!(diagnostics
        .iter()
        .all(|path| backend.cleanup_recovery_anchor(path).is_none()));
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let first = restarted.startup_recovery_paths_for_test();
    assert!(first.len() >= 2);
    assert!(first
        .iter()
        .all(|path| restarted.cleanup_recovery_anchor(path).is_none()));
    assert_eq!(fs::read(&owned).unwrap(), b"owned");
    assert_eq!(fs::read(&alias).unwrap(), b"owned");
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(second_restart.startup_recovery_paths_for_test(), first);
    assert_eq!(fs::read(owned).unwrap(), b"owned");
    assert_eq!(fs::read(alias).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn review20_replaced_owned_candidate_never_receives_cleanup_authority() {
    let root = tempdir().unwrap();
    let retained_paths = Arc::new(std::sync::Mutex::new(None));
    let observed = retained_paths.clone();
    let swapped = Arc::new(AtomicBool::new(false));
    let swap_once = swapped.clone();
    let backend = Arc::new(
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_after_directory_create_before_anchor({
                let observed = observed.clone();
                move |private| {
                    let retained = private.with_extension("review20-owned");
                    fs::rename(private, &retained).unwrap();
                    fs::create_dir(private).unwrap();
                    *observed.lock().unwrap() =
                        Some((private.to_path_buf(), retained, PathBuf::new()));
                }
            })
            .with_before_owned_candidate_anchor_open(move |candidate| {
                if swap_once.swap(true, Ordering::SeqCst) {
                    return;
                }
                let original = candidate.with_extension("review20-original");
                fs::rename(candidate, &original).unwrap();
                fs::create_dir(candidate).unwrap();
                let mut paths = retained_paths.lock().unwrap();
                let (foreign, retained, _) = paths.clone().unwrap();
                *paths = Some((foreign, retained, original));
            }),
    );

    assert!(
        backend
            .create_dir_with_ownership_anchor(Path::new("/visible"))
            .is_err(),
        "the create-to-anchor replacement must fail closed"
    );
    let (first_foreign, replaced_candidate, original) = observed.lock().unwrap().clone().unwrap();
    assert!(swapped.load(Ordering::SeqCst));
    let recovery_paths = backend.startup_recovery_paths_for_test();
    assert!(!recovery_paths.is_empty());
    assert!(
        recovery_paths
            .iter()
            .all(|path| backend.cleanup_recovery_anchor(path).is_none()),
        "a replacement must never receive cleanup authority"
    );
    let error = crate::sftp_manager::transfer_job::startup_backend_recovery_error(
        backend.clone(),
        recovery_paths,
    );
    let recovery_id = error.recovery_id().expect("retry ID");
    assert!(
        crate::sftp_manager::transfer_job::retry_recovery(recovery_id).is_err(),
        "a replacement must remain unresolved instead of being deleted"
    );
    assert!(replaced_candidate.is_dir());
    assert!(first_foreign.is_dir());
    assert!(original.is_dir());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .iter()
        .all(|path| restarted.cleanup_recovery_anchor(path).is_none()));
    assert!(replaced_candidate.is_dir());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_review20_placeholder_isolation_uses_exchange(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/placeholder")
    } else {
        Path::new("/placeholder.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let retained = Arc::new(std::sync::Mutex::new(None));
    let observed = retained.clone();
    let source_present_during_classification = Arc::new(AtomicBool::new(false));
    let source_present = source_present_during_classification.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_before_placeholder_isolation(move |source| {
            let owned = source.with_extension("review20-owned");
            fs::rename(source, &owned).unwrap();
            if directory {
                fs::create_dir(source).unwrap();
            } else {
                fs::write(source, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some(owned);
        })
        .with_after_placeholder_isolation_before_classification(move |source, _| {
            source_present.store(source.exists(), Ordering::SeqCst);
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .cleanup_isolation_placeholder(path, anchor, &identity)
        .expect_err("a source replacement at the isolation boundary must be recoverable");
    assert!(
        source_present_during_classification.load(Ordering::SeqCst),
        "atomic exchange isolation must keep a held placeholder at the public path"
    );
    if directory {
        assert!(local.is_dir());
    } else {
        assert_eq!(fs::read(&local).unwrap(), b"foreign");
    }
    let retained = retained.lock().unwrap().clone().unwrap();
    let retained_path = PathBuf::from("/").join(retained.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&retained_path));
    assert!(restarted.cleanup_recovery_anchor(&retained_path).is_some());
    if directory {
        assert!(local.is_dir());
    } else {
        assert_eq!(fs::read(local).unwrap(), b"foreign");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review20_file_placeholder_isolation_uses_exchange() {
    assert_review20_placeholder_isolation_uses_exchange(false);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review20_directory_placeholder_isolation_uses_exchange() {
    assert_review20_placeholder_isolation_uses_exchange(true);
}

#[cfg(unix)]
#[test]
fn review20_sibling_move_before_registry_commit_is_rediscovered() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    fs::write(&owned, b"owned").unwrap();
    let moved = Arc::new(std::sync::Mutex::new(None));
    let observed = moved.clone();
    let move_once = Arc::new(AtomicBool::new(false));
    let moved_once = move_once.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_before_sibling_registry_commit(move |_, physical| {
            if moved_once.swap(true, Ordering::SeqCst) {
                return;
            }
            let destination = physical.with_extension("review20-moved");
            fs::rename(physical, &destination).unwrap();
            fs::write(physical, b"foreign").unwrap();
            *observed.lock().unwrap() = Some(destination);
        });
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .unwrap();
    let moved = moved.lock().unwrap().clone().unwrap();
    let moved_path = PathBuf::from("/").join(moved.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&moved_path));
    assert!(restarted.cleanup_recovery_anchor(&moved_path).is_some());
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(second_restart
        .startup_recovery_paths_for_test()
        .contains(&moved_path));
    assert_eq!(fs::read(moved).unwrap(), b"owned");
    assert_eq!(fs::read(owned).unwrap(), b"foreign");
}

#[cfg(unix)]
#[test]
fn review20_opaque_guarded_rename_persists_physical_path_before_crash() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_rename(|_, _| panic!("review20 crash immediately after opaque guarded rename"));
    let namespace = backend
        .directory_reservation_namespace(root.path())
        .unwrap();
    let physical = namespace.path.join("review20-opaque.bin");
    fs::write(&physical, b"owned").unwrap();
    let identity = stable_identity_from_local_metadata(&fs::symlink_metadata(&physical).unwrap());
    let logical = backend
        .persist_unresolved_physical_candidate("review20-opaque-source", &physical, Some(identity))
        .unwrap();
    let target = PathBuf::from("/.review20.zaplex-source-opaque");
    backend
        .map_opaque_cleanup_sibling(&logical, &target)
        .unwrap();
    let target_physical = namespace
        .path
        .join(target.file_name().expect("target file name"));

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.rename(&logical, &target)
    }));
    assert!(crashed.is_err());
    assert!(target_physical.exists());
    drop(backend);

    let restarted = Arc::new(InMemorySftpBackend::new(root.path().to_path_buf()));
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&target));
    assert!(restarted.cleanup_recovery_anchor(&target).is_some());
    let error = crate::sftp_manager::transfer_job::startup_backend_recovery_error(
        restarted.clone(),
        vec![target.clone()],
    );
    crate::sftp_manager::transfer_job::retry_recovery(error.recovery_id().unwrap()).unwrap();
    assert!(!target_physical.exists());
}

#[cfg(unix)]
#[test]
fn review20_cross_directory_hardlink_is_never_uniquely_owned() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("first")).unwrap();
    fs::create_dir(root.path().join("second")).unwrap();
    let owned = root.path().join("first/owned.bin");
    let alias = root.path().join("second/alias.bin");
    fs::write(&owned, b"owned").unwrap();
    fs::hard_link(&owned, &alias).unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/first/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    let paths = backend
        .persist_anchor_sibling_recovery(
            Path::new("/first/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .unwrap();
    assert!(paths
        .iter()
        .all(|path| backend.cleanup_recovery_anchor(path).is_none()));
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .iter()
        .all(|path| restarted.cleanup_recovery_anchor(path).is_none()));
    assert_eq!(fs::read(owned).unwrap(), b"owned");
    assert_eq!(fs::read(alias).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn review20_transient_sibling_scan_failure_retries_to_completion() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    fs::write(&owned, b"owned").unwrap();
    let backend = Arc::new(
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_sibling_recovery_failure(SiblingRecoveryFailure::ReadDirectory),
    );
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();
    let paths = backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .unwrap();
    backend.clear_sibling_recovery_failure();

    let error =
        crate::sftp_manager::transfer_job::startup_backend_recovery_error(backend.clone(), paths);
    crate::sftp_manager::transfer_job::retry_recovery(error.recovery_id().unwrap())
        .expect("retry must rescan after a transient directory failure");
    assert!(!owned.exists());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted.startup_recovery_paths_for_test().is_empty());
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(second_restart.startup_recovery_paths_for_test().is_empty());
}

#[cfg(unix)]
fn assert_review21_rescan_anchor_never_binds_replacement(directory: bool) {
    let root = tempdir().unwrap();
    let physical = root
        .path()
        .join(if directory { "owned" } else { "owned.bin" });
    if directory {
        fs::create_dir(&physical).unwrap();
    } else {
        fs::write(&physical, b"same").unwrap();
    }
    let retained = physical.with_extension("review21-retained");
    let swapped = Arc::new(AtomicBool::new(false));
    let swap_once = swapped.clone();
    let backend = Arc::new(
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_before_sibling_recovery_anchor_open(move |candidate| {
                if swap_once.swap(true, Ordering::SeqCst) {
                    return;
                }
                fs::rename(candidate, &retained).unwrap();
                if directory {
                    fs::create_dir(candidate).unwrap();
                } else {
                    fs::write(candidate, b"same").unwrap();
                }
            }),
    );
    let owned_path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let expected = backend
        .existing_entry_ownership_anchor(owned_path)
        .unwrap()
        .unwrap()
        .identity()
        .unwrap();
    let rescan = backend
        .persist_sibling_rescan_record(
            root.path(),
            &expected,
            "owned-isolation-source",
            "review21 replacement window",
        )
        .unwrap();

    backend
        .retry_unresolved_recovery(&rescan)
        .expect_err("a replacement opened after classification must remain unresolved");
    assert!(swapped.load(Ordering::SeqCst));
    if directory {
        assert!(physical.is_dir());
    } else {
        assert_eq!(fs::read(&physical).unwrap(), b"same");
    }
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .iter()
        .all(|path| restarted.cleanup_recovery_anchor(path).is_none()));
    if directory {
        assert!(physical.is_dir());
    } else {
        assert_eq!(fs::read(physical).unwrap(), b"same");
    }
}

#[cfg(unix)]
#[test]
fn review21_file_rescan_anchor_never_binds_replacement() {
    assert_review21_rescan_anchor_never_binds_replacement(false);
}

#[cfg(unix)]
#[test]
fn review21_directory_rescan_anchor_never_binds_replacement() {
    assert_review21_rescan_anchor_never_binds_replacement(true);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_review21_public_tombstone_swap_is_restored(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let swapped_path = Arc::new(std::sync::Mutex::new(None));
    let observed = swapped_path.clone();
    let calls = Arc::new(AtomicU64::new(0));
    let call_count = calls.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_before_private_placeholder_isolation(move |public, _| {
            if call_count.fetch_add(1, Ordering::SeqCst) != 0 {
                return;
            }
            let retained = public.with_extension("review21-owned");
            fs::rename(public, &retained).unwrap();
            if directory {
                fs::create_dir(public).unwrap();
            } else {
                fs::write(public, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some((public.to_path_buf(), retained));
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .cleanup_isolation_placeholder(path, anchor, &identity)
        .expect_err("a tombstone replacement at the isolation syscall must fail closed");
    let (foreign, retained) = swapped_path
        .lock()
        .unwrap()
        .clone()
        .expect("outer tombstone cutpoint must run");
    if directory {
        assert!(foreign.is_dir(), "the foreign directory must remain public");
    } else {
        assert_eq!(
            fs::read(&foreign).unwrap(),
            b"foreign",
            "the foreign file must remain public"
        );
    }
    assert!(
        retained.exists(),
        "the owned tombstone must remain reachable"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!restarted.startup_recovery_paths_for_test().is_empty());
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review21_file_public_tombstone_swap_is_restored() {
    assert_review21_public_tombstone_swap_is_restored(false);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review21_directory_public_tombstone_swap_is_restored() {
    assert_review21_public_tombstone_swap_is_restored(true);
}

#[cfg(unix)]
#[test]
fn review21_sibling_move_is_persisted_before_rescan_crash() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    fs::write(&owned, b"owned").unwrap();
    let moved = Arc::new(std::sync::Mutex::new(None));
    let observed = moved.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_sibling_registry_write(move |_, physical| {
            let destination = physical.with_extension("review21-moved");
            fs::rename(physical, &destination).unwrap();
            fs::write(physical, b"foreign").unwrap();
            *observed.lock().unwrap() = Some(destination);
        })
        .with_before_sibling_rescan_iteration(|_| {
            panic!("review21 crash after durable rescan transition")
        });
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend
            .persist_anchor_sibling_recovery(
                Path::new("/owned.bin"),
                anchor,
                &identity,
                "owned-isolation-source",
            )
            .unwrap()
    }));
    assert!(crashed.is_err());
    let moved = moved.lock().unwrap().clone().unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(
        !restarted.startup_recovery_paths_for_test().is_empty(),
        "the sibling rescan transition must survive the crash cutpoint"
    );
    assert_eq!(fs::read(&moved).unwrap(), b"owned");
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!second_restart.startup_recovery_paths_for_test().is_empty());
    assert_eq!(fs::read(moved).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn review21_actual_registry_write_failure_is_not_synthetic_success() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    fs::write(&owned, b"owned").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();
    backend.fail_artifact_registry_writes_for_test();

    let error = backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .expect_err("the real registry write failure must propagate");
    assert!(
        error.to_string().contains("No space left") || error.to_string().contains("os error 28"),
        "the real write error must remain visible: {error}"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(
        restarted.startup_recovery_paths_for_test().is_empty(),
        "a failed durable write must not be reported as durable recovery"
    );
    assert_eq!(fs::read(owned).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn review21_late_hardlink_blocks_destructive_retry() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("other")).unwrap();
    let owned = root.path().join("owned.bin");
    let alias = root.path().join("other/alias.bin");
    fs::write(&owned, b"owned").unwrap();
    let backend = Arc::new(InMemorySftpBackend::new(root.path().to_path_buf()));
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();
    let paths = backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .unwrap();
    fs::hard_link(&owned, &alias).unwrap();

    let error =
        crate::sftp_manager::transfer_job::startup_backend_recovery_error(backend.clone(), paths);
    assert!(
        crate::sftp_manager::transfer_job::retry_recovery(error.recovery_id().unwrap()).is_err(),
        "a newly introduced hardlink must revoke cleanup authorization"
    );
    assert_eq!(fs::read(&owned).unwrap(), b"owned");
    assert_eq!(fs::read(&alias).unwrap(), b"owned");
}

#[cfg(unix)]
fn assert_review21_private_child_replacement_is_not_unlinked(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let replacement = Arc::new(std::sync::Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_before_private_placeholder_unlink(move |_, private| {
            let retained = private.with_extension("review21-retained");
            fs::rename(private, &retained).unwrap();
            if directory {
                fs::create_dir(private).unwrap();
            } else {
                fs::write(private, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some((private.to_path_buf(), retained));
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .cleanup_isolation_placeholder(path, anchor, &identity)
        .expect_err("a private child replacement must fail closed");
    let (foreign, retained) = replacement.lock().unwrap().clone().unwrap();
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
    }
    assert!(retained.exists());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!restarted.startup_recovery_paths_for_test().is_empty());
    if directory {
        assert!(foreign.is_dir());
    } else {
        assert_eq!(fs::read(foreign).unwrap(), b"foreign");
    }
}

#[cfg(unix)]
#[test]
fn review21_private_file_child_replacement_is_not_unlinked() {
    assert_review21_private_child_replacement_is_not_unlinked(false);
}

#[cfg(unix)]
#[test]
fn review21_private_directory_child_replacement_is_not_unlinked() {
    assert_review21_private_child_replacement_is_not_unlinked(true);
}

#[cfg(unix)]
fn assert_review22_public_cleanup_never_deletes_replacement(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let replacement = Arc::new(std::sync::Mutex::new(None));
    let observed = replacement.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_guarded_cleanup_verification(move |tombstone| {
            let retained = tombstone.with_extension("review22-owned");
            fs::rename(tombstone, &retained).unwrap();
            if directory {
                fs::create_dir(tombstone).unwrap();
            } else {
                fs::write(tombstone, b"foreign").unwrap();
            }
            *observed.lock().unwrap() = Some((tombstone.to_path_buf(), retained));
        });
    let identity = backend.stable_identity(path).unwrap();
    let result = if directory {
        backend.delete_empty_dir_if_matches(path, &identity)
    } else {
        backend.delete_file_if_matches(path, &identity, &format!("{:x}", Sha256::digest(b"owned")))
    };

    assert!(
        result.is_err(),
        "a public replacement after verification must remain unresolved"
    );
    let (foreign, retained) = replacement.lock().unwrap().clone().unwrap();
    if directory {
        assert!(foreign.is_dir(), "the foreign directory must remain");
        assert!(
            retained.is_dir(),
            "the owned directory must remain recoverable"
        );
    } else {
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read(&retained).unwrap(), b"owned");
    }
}

#[cfg(unix)]
#[test]
fn review22_public_file_cleanup_isolates_before_final_delete() {
    assert_review22_public_cleanup_never_deletes_replacement(false);
}

#[cfg(unix)]
#[test]
fn review22_public_directory_cleanup_isolates_before_final_delete() {
    assert_review22_public_cleanup_never_deletes_replacement(true);
}

#[cfg(unix)]
#[test]
fn review22_hardlink_added_after_verification_blocks_cleanup() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("other")).unwrap();
    let source = root.path().join("owned.bin");
    let alias = root.path().join("other/alias.bin");
    fs::write(&source, b"owned").unwrap();
    let alias_for_hook = alias.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_guarded_cleanup_verification(move |tombstone| {
            fs::hard_link(tombstone, &alias_for_hook).unwrap();
        });
    let identity = backend.stable_identity(Path::new("/owned.bin")).unwrap();

    backend
        .delete_file_if_matches(
            Path::new("/owned.bin"),
            &identity,
            &format!("{:x}", Sha256::digest(b"owned")),
        )
        .expect_err("a hardlink added after verification must revoke cleanup authority");

    assert_eq!(fs::read(source).unwrap(), b"owned");
    assert_eq!(fs::read(alias).unwrap(), b"owned");
}

#[cfg(unix)]
#[test]
fn review22_candidate_move_survives_rescan_transition_failure() {
    let root = tempdir().unwrap();
    let owned = root.path().join("owned.bin");
    fs::write(&owned, b"owned").unwrap();
    let moved = Arc::new(std::sync::Mutex::new(None));
    let observed = moved.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_sibling_registry_write(move |_, physical| {
            let destination = physical.with_extension("review22-moved");
            fs::rename(physical, &destination).unwrap();
            fs::write(physical, b"foreign").unwrap();
            *observed.lock().unwrap() = Some(destination);
        });
    backend.fail_artifact_registry_transitions_for_test();
    let anchor = backend
        .existing_entry_ownership_anchor(Path::new("/owned.bin"))
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    backend
        .persist_anchor_sibling_recovery(
            Path::new("/owned.bin"),
            anchor,
            &identity,
            "owned-isolation-source",
        )
        .expect_err("a failed candidate-to-rescan transition must remain a real error");
    let moved = moved.lock().unwrap().clone().unwrap();
    let moved_path = PathBuf::from("/").join(moved.strip_prefix(root.path()).unwrap());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let rescan_paths = restarted.startup_recovery_paths_for_test();
    let resolved = rescan_paths
        .iter()
        .find_map(|path| restarted.retry_unresolved_recovery(path).ok().flatten())
        .expect("the durable parent-rescan record must resolve the moved object");
    assert!(resolved.contains(&moved_path));
    assert_eq!(fs::read(&moved).unwrap(), b"owned");
    assert_eq!(fs::read(&owned).unwrap(), b"foreign");
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(second_restart
        .startup_recovery_paths_for_test()
        .contains(&moved_path));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_review22_exchange_cutpoint_is_restart_recoverable(directory: bool) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_placeholder_isolation_before_classification(|_, _| {
            panic!("review22 crash after exchange before phase transition")
        });
    let anchor = backend
        .existing_entry_ownership_anchor(path)
        .unwrap()
        .unwrap();
    let identity = anchor.identity().unwrap();

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.cleanup_isolation_placeholder(path, anchor, &identity)
    }));
    assert!(crashed.is_err());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let recovery_paths = restarted.startup_recovery_paths_for_test();
    assert!(!recovery_paths.is_empty());
    assert!(
        recovery_paths.iter().any(|recovery| {
            restarted
                .retry_unresolved_recovery(recovery)
                .is_ok_and(|replacement| replacement.is_some())
        }),
        "the persisted pre-exchange state must classify both candidates after restart"
    );
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!second_restart.startup_recovery_paths_for_test().is_empty());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_file_exchange_cutpoint_is_restart_recoverable() {
    assert_review22_exchange_cutpoint_is_restart_recoverable(false);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_directory_exchange_cutpoint_is_restart_recoverable() {
    assert_review22_exchange_cutpoint_is_restart_recoverable(true);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn assert_review22_private_unlink_cutpoint_is_restart_recoverable(
    directory: bool,
    crash_after: usize,
) {
    let root = tempdir().unwrap();
    let path = if directory {
        Path::new("/owned")
    } else {
        Path::new("/owned.bin")
    };
    let local = root.path().join(path.strip_prefix("/").unwrap());
    if directory {
        fs::create_dir(&local).unwrap();
    } else {
        fs::write(&local, b"owned").unwrap();
    }
    let unlink_count = Arc::new(AtomicUsize::new(0));
    let observed = unlink_count.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_private_namespace_unlink(move |_| {
            if observed.fetch_add(1, Ordering::SeqCst) == crash_after {
                panic!("review22 crash at private unlink cutpoint")
            }
        });
    let identity = backend.stable_identity(path).unwrap();

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if directory {
            backend.delete_empty_dir_if_matches(path, &identity)
        } else {
            backend.delete_file_if_matches(
                path,
                &identity,
                &format!("{:x}", Sha256::digest(b"owned")),
            )
        }
    }));
    assert!(crashed.is_err());
    assert!(!local.exists(), "the public path must stay absent");
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    if crash_after == 1 {
        let private_exchange = restarted
            .startup_recovery_paths_for_test()
            .into_iter()
            .find(|path| path.to_string_lossy().contains("private-cleanup-exchange"))
            .expect("the private exchange must remain durable");
        let replacements = restarted
            .retry_unresolved_recovery(&private_exchange)
            .expect("the sentinel-retired phase must be classifiable")
            .expect("the owned private candidate must remain retryable");
        let recovery_path = replacements
            .into_iter()
            .find(|path| restarted.cleanup_recovery_anchor(path).is_some())
            .expect("the owned private candidate must retain cleanup authority");
        drop(restarted);

        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert!(second_restart
            .startup_recovery_paths_for_test()
            .contains(&recovery_path));
        assert!(second_restart
            .cleanup_recovery_anchor(&recovery_path)
            .is_some());
        return;
    }

    let mut attempts = Vec::new();
    for _ in 0..3 {
        for recovery in restarted.startup_recovery_paths_for_test() {
            let result = restarted.retry_unresolved_recovery(&recovery);
            attempts.push(format!("{}: {result:?}", recovery.display()));
        }
    }
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    let remaining = second_restart.startup_recovery_paths_for_test();
    assert!(
        remaining.is_empty(),
        "an applied private-delete record must converge after restart: {remaining:?}; {attempts:?}"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_file_private_unlink_cutpoint_is_restart_recoverable() {
    assert_review22_private_unlink_cutpoint_is_restart_recoverable(false, 2);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_directory_private_unlink_cutpoint_is_restart_recoverable() {
    assert_review22_private_unlink_cutpoint_is_restart_recoverable(true, 2);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_file_sentinel_retirement_cutpoint_is_restart_recoverable() {
    assert_review22_private_unlink_cutpoint_is_restart_recoverable(false, 1);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn review22_directory_sentinel_retirement_cutpoint_is_restart_recoverable() {
    assert_review22_private_unlink_cutpoint_is_restart_recoverable(true, 1);
}

#[cfg(unix)]
#[test]
fn review23_partial_artifact_write_failure_keeps_exchange_durable() {
    let root = tempdir().unwrap();
    let first = root.path().join("first.bin");
    let second = root.path().join("second.bin");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_artifact_registry_write_failure_on(2);
    let exchange_path =
        InMemorySftpBackend::unresolved_registry_path("review23-exchange", "first:second");
    let record = PersistentExchangeRecord {
        path: exchange_path.clone(),
        first: PersistentExchangeCandidate {
            physical_path: first,
            role: "first-owned".to_string(),
            identity: Some(backend.stable_identity(Path::new("/first.bin")).unwrap()),
        },
        second: PersistentExchangeCandidate {
            physical_path: second,
            role: "second-owned".to_string(),
            identity: Some(backend.stable_identity(Path::new("/second.bin")).unwrap()),
        },
        phase: PersistentExchangePhase::Prepared,
        generation: uuid::Uuid::new_v4().to_string(),
        legacy: false,
    };
    backend.persist_exchange_record(record.clone()).unwrap();

    backend
        .resolve_persistent_exchange(&record)
        .expect_err("the second replacement-record ENOSPC must remain visible");
    assert!(
        backend
            .startup_recovery_paths_for_test()
            .contains(&exchange_path),
        "the sole exchange record must remain durable after a partial replacement write"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&exchange_path));
    let replacements = restarted
        .retry_unresolved_recovery(&exchange_path)
        .unwrap()
        .expect("the durable exchange must converge idempotently");
    assert_eq!(replacements.len(), 2);
    drop(restarted);

    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    for replacement in replacements {
        assert!(second_restart
            .startup_recovery_paths_for_test()
            .contains(&replacement));
        assert!(second_restart
            .cleanup_recovery_anchor(&replacement)
            .is_some());
    }
}

#[cfg(unix)]
#[test]
fn review23_in_tree_namespace_is_hidden_during_creation() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("visible.bin"), b"visible").unwrap();
    let created = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let backend = Arc::new(
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_in_tree_directory_reservation_namespace()
            .with_after_namespace_create_before_anchor({
                let created = created.clone();
                let resume = resume.clone();
                move |_| {
                    created.wait();
                    resume.wait();
                }
            }),
    );
    let worker_backend = backend.clone();
    let worker = std::thread::spawn(move || {
        worker_backend.create_dir_with_ownership_anchor(Path::new("/stage"))
    });
    created.wait();

    let namespace_name = backend.directory_reservation_namespace_name();
    let namespace_path = PathBuf::from("/").join(&namespace_name);
    let listed = backend.list_dir(Path::new("/")).unwrap();
    assert!(listed.iter().any(|entry| entry.name == "visible.bin"));
    assert!(
        listed.iter().all(|entry| entry.name != namespace_name),
        "the exact private namespace must never be publicly enumerable"
    );
    assert!(
        backend.lstat(&namespace_path).is_err(),
        "the exact private namespace must be rejected before registration completes"
    );

    resume.wait();
    worker.join().unwrap().unwrap();
}

#[cfg(unix)]
#[test]
fn review23_terminal_artifact_retirement_releases_stale_anchors() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());

    let deleted = Path::new("/.owned.zaplex-transfer-delete");
    fs::write(root.path().join(".owned.zaplex-transfer-delete"), b"owned").unwrap();
    let deleted_anchor = backend
        .existing_entry_ownership_anchor(deleted)
        .unwrap()
        .unwrap();
    backend
        .persist_artifact_identity(deleted, deleted_anchor)
        .unwrap();
    backend.delete_file(deleted).unwrap();
    assert!(backend.cleanup_recovery_anchor(deleted).is_none());
    fs::write(
        root.path().join(".owned.zaplex-transfer-delete"),
        b"foreign",
    )
    .unwrap();
    assert!(
        backend.cleanup_recovery_anchor(deleted).is_none(),
        "a replacement must never observe retired cleanup authorization"
    );

    let renamed = Path::new("/.owned.zaplex-transfer-rename");
    fs::write(root.path().join(".owned.zaplex-transfer-rename"), b"owned").unwrap();
    let renamed_anchor = backend
        .existing_entry_ownership_anchor(renamed)
        .unwrap()
        .unwrap();
    backend
        .persist_artifact_identity(renamed, renamed_anchor)
        .unwrap();
    backend.rename(renamed, Path::new("/renamed.bin")).unwrap();
    assert!(backend.cleanup_recovery_anchor(renamed).is_none());

    let recovered = Path::new("/.owned.zaplex-transfer-recovery");
    fs::write(
        root.path().join(".owned.zaplex-transfer-recovery"),
        b"owned",
    )
    .unwrap();
    let recovered_anchor = backend
        .existing_entry_ownership_anchor(recovered)
        .unwrap()
        .unwrap();
    backend
        .persist_artifact_identity(recovered, recovered_anchor)
        .unwrap();
    backend.release_cleanup_recovery_path(recovered).unwrap();
    assert!(backend.cleanup_recovery_anchor(recovered).is_none());
}

fn prepare_review24_cleanup_record(backend: &InMemorySftpBackend, root: &Path) -> PathBuf {
    let path = PathBuf::from("/.owned.zaplex-delete-review24");
    fs::write(root.join(".owned.zaplex-delete-review24"), b"owned").unwrap();
    let anchor = backend
        .existing_entry_ownership_anchor(&path)
        .unwrap()
        .unwrap();
    backend.persist_artifact_identity(&path, anchor).unwrap();
    assert!(backend.cleanup_recovery_anchor(&path).is_some());
    path
}

#[cfg(unix)]
#[test]
fn review24_artifact_removal_failure_preserves_retry_authority() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let path = prepare_review24_cleanup_record(&backend, root.path());
    backend.fail_artifact_registry_removals_for_test();

    backend
        .release_cleanup_recovery_path(&path)
        .expect_err("registry removal ENOSPC must remain visible");

    assert!(
        backend.cleanup_recovery_anchor(&path).is_some(),
        "failed durable retirement must preserve the cleanup anchor"
    );
    assert!(backend.startup_recovery_paths_for_test().contains(&path));
}

#[cfg(unix)]
#[test]
fn review24_artifact_retirement_fsync_failure_preserves_retry_authority() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let path = prepare_review24_cleanup_record(&backend, root.path());
    backend.fail_artifact_registry_retirement_sync_for_test();

    backend
        .release_cleanup_recovery_path(&path)
        .expect_err("registry root fsync EIO must remain visible");

    assert!(
        backend.cleanup_recovery_anchor(&path).is_some(),
        "uncertain durable retirement must preserve the cleanup anchor"
    );
    assert!(backend.startup_recovery_paths_for_test().contains(&path));
}

#[cfg(unix)]
#[test]
fn review24_equivalent_replacement_generation_blocks_aba_retirement() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let path = prepare_review24_cleanup_record(&backend, root.path());
    backend.replace_artifact_before_retirement_for_test();

    backend
        .release_cleanup_recovery_path(&path)
        .expect_err("an equivalent newer generation must defeat stale retirement");

    assert!(backend.cleanup_recovery_anchor(&path).is_some());
    assert!(backend.startup_recovery_paths_for_test().contains(&path));
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted.startup_recovery_paths_for_test().contains(&path));
}

#[cfg(unix)]
#[test]
fn review24_namespace_probe_errors_preserve_authenticated_record_across_restarts() {
    for failure in [
        NamespaceProbeFailure::Record,
        NamespaceProbeFailure::NamespacePath,
        NamespaceProbeFailure::Parent,
    ] {
        let root = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_in_tree_directory_reservation_namespace();
        backend
            .create_dir_with_ownership_anchor(Path::new("/first-stage"))
            .unwrap();
        let namespace = backend
            .directory_reservation_namespace_path_for_test()
            .unwrap();
        let original_record = backend.namespace_record_contents_for_test();
        assert_eq!(original_record.len(), 1);
        drop(backend);

        let failing = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_in_tree_directory_reservation_namespace()
            .with_namespace_probe_failure(failure);
        assert!(
            failing
                .create_dir_with_ownership_anchor(Path::new("/second-stage"))
                .is_err(),
            "EACCES/EIO namespace probes must fail closed"
        );
        assert_eq!(
            failing.namespace_record_contents_for_test(),
            original_record,
            "a failed probe must not replace the authenticated namespace record"
        );
        drop(failing);

        let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(
            restarted.directory_reservation_namespace_path_for_test(),
            Some(namespace.clone())
        );
        assert_eq!(
            restarted.namespace_record_contents_for_test(),
            original_record
        );
        drop(restarted);

        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(
            second_restart.directory_reservation_namespace_path_for_test(),
            Some(namespace)
        );
        assert_eq!(
            second_restart.namespace_record_contents_for_test(),
            original_record
        );
    }
}

fn assert_review24_foreign_namespace_collision_remains_public(directory: bool) {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    let namespace_name = backend.directory_reservation_namespace_name();
    let namespace_local = root.path().join(&namespace_name);
    if directory {
        fs::create_dir(&namespace_local).unwrap();
    } else {
        fs::write(&namespace_local, b"foreign").unwrap();
    }

    assert!(
        backend
            .create_dir_with_ownership_anchor(Path::new("/stage"))
            .is_err(),
        "a foreign namespace collision must fail authentication"
    );

    assert!(
        backend
            .list_dir(Path::new("/"))
            .unwrap()
            .iter()
            .any(|entry| entry.name == namespace_name),
        "a failed scoped reservation must not hide the foreign collision"
    );
    assert!(
        backend
            .lstat(&PathBuf::from("/").join(&namespace_name))
            .is_ok(),
        "a failed scoped reservation must not block public access to the foreign collision"
    );
}

#[cfg(unix)]
#[test]
fn review24_foreign_file_namespace_collision_remains_public() {
    assert_review24_foreign_namespace_collision_remains_public(false);
}

#[cfg(unix)]
#[test]
fn review24_foreign_directory_namespace_collision_remains_public() {
    assert_review24_foreign_namespace_collision_remains_public(true);
}

#[cfg(unix)]
#[test]
fn review24_successful_namespace_reservation_stays_hidden_during_registration() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("visible.bin"), b"visible").unwrap();
    let created = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let backend = Arc::new(
        InMemorySftpBackend::new(root.path().to_path_buf())
            .with_in_tree_directory_reservation_namespace()
            .with_after_namespace_create_before_anchor({
                let created = created.clone();
                let resume = resume.clone();
                move |_| {
                    created.wait();
                    resume.wait();
                }
            }),
    );
    let worker_backend = backend.clone();
    let worker = std::thread::spawn(move || {
        worker_backend.create_dir_with_ownership_anchor(Path::new("/stage"))
    });
    created.wait();

    let namespace_name = backend.directory_reservation_namespace_name();
    assert!(backend
        .list_dir(Path::new("/"))
        .unwrap()
        .iter()
        .all(|entry| entry.name != namespace_name));
    assert!(backend
        .lstat(&PathBuf::from("/").join(&namespace_name))
        .is_err());

    resume.wait();
    worker.join().unwrap().unwrap();
    let entries = backend.list_dir(Path::new("/")).unwrap();
    assert!(entries.iter().any(|entry| entry.name == "visible.bin"));
    assert!(entries.iter().all(|entry| entry.name != namespace_name));
}

fn artifact_registry_record_count(backend: &InMemorySftpBackend) -> usize {
    let registry = backend
        .directory_reservation_registry
        .as_ref()
        .expect("test registry must exist");
    fs::read_dir(&registry.root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("artifact-") && name.ends_with(".record")
        })
        .count()
}

fn namespace_temporary_record_count(backend: &InMemorySftpBackend) -> usize {
    let registry = backend
        .directory_reservation_registry
        .as_ref()
        .expect("test registry must exist");
    fs::read_dir(&registry.root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(".namespace-") && name.ends_with(".tmp")
        })
        .count()
}

fn legacy_payload(payload: &str, legacy_version: &str) -> String {
    std::iter::once(legacy_version.to_string())
        .chain(payload.lines().skip(1).filter_map(|line| {
            (!line.starts_with("generation=") && !line.starts_with("retired="))
                .then(|| line.to_string())
        }))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(unix)]
#[test]
fn review25_new_generation_survives_post_retirement_cleanup() {
    let root = tempdir().unwrap();
    let marker_path = root.path().join(".review25-new-generation-marker");
    let marker_path_for_hook = marker_path.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_artifact_retirement_generation_check(move |backend, path| {
            let anchor = backend
                .existing_entry_ownership_anchor(path)
                .unwrap()
                .unwrap();
            let identity = anchor.identity().unwrap();
            let physical_path = backend.to_local(path).unwrap();
            let record = PersistentArtifactRecord::active(
                path.to_path_buf(),
                Some(physical_path.clone()),
                "cleanup-tombstone".to_string(),
                Some(identity.clone()),
            );
            let registry = backend.directory_reservation_registry.as_ref().unwrap();
            registry.write_artifact_record(&record).unwrap();
            backend
                .persistent_artifact_records
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), record);
            backend
                .cleanup_recovery_identities
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), (identity, anchor));
            backend
                .opaque_recovery_paths
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), physical_path);

            fs::write(&marker_path_for_hook, b"new generation marker").unwrap();
            let marker_file = open_local_cleanup_anchor(&marker_path_for_hook).unwrap();
            let marker_anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
                file: marker_file,
                root: backend.root.clone(),
                opaque_paths: Some(backend.opaque_recovery_paths.clone()),
            });
            backend.opaque_recovery_markers.lock().unwrap().insert(
                path.to_path_buf(),
                OwnedReservationMarker {
                    path: marker_path_for_hook.clone(),
                    identity: marker_anchor.identity().unwrap(),
                    anchor: marker_anchor,
                },
            );
        });
    let path = prepare_review24_cleanup_record(&backend, root.path());

    let _ = backend.release_cleanup_recovery_path(&path);

    assert!(
        backend.cleanup_recovery_anchor(&path).is_some(),
        "retiring an older generation must not remove the newer anchor"
    );
    assert!(
        backend
            .opaque_recovery_paths
            .lock()
            .unwrap()
            .contains_key(&path),
        "retiring an older generation must not remove the newer physical mapping"
    );
    assert!(
        backend
            .opaque_recovery_markers
            .lock()
            .unwrap()
            .contains_key(&path),
        "retiring an older generation must not remove the newer marker"
    );
    assert!(marker_path.exists());
    assert_eq!(artifact_registry_record_count(&backend), 1);
}

#[cfg(unix)]
#[test]
fn review25_terminal_retirement_keeps_registry_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());

    for index in 0..64 {
        let path = PathBuf::from(format!("/.owned.zaplex-transfer-review25-{index}"));
        fs::write(
            root.path()
                .join(path.file_name().expect("artifact must have a name")),
            b"owned",
        )
        .unwrap();
        let anchor = backend
            .existing_entry_ownership_anchor(&path)
            .unwrap()
            .unwrap();
        backend.persist_artifact_identity(&path, anchor).unwrap();
        backend.release_cleanup_recovery_path(&path).unwrap();
    }

    assert_eq!(
        artifact_registry_record_count(&backend),
        0,
        "terminal artifact retirement must not grow one record per transfer"
    );
}

#[derive(Clone, Copy)]
enum Review25RetirementFailure {
    Transition,
    Unlink,
    FinalSync,
}

#[cfg(unix)]
#[test]
fn review25_retirement_failures_converge_without_losing_retry_authority() {
    for failure in [
        Review25RetirementFailure::Transition,
        Review25RetirementFailure::Unlink,
        Review25RetirementFailure::FinalSync,
    ] {
        let root = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf());
        let path = prepare_review24_cleanup_record(&backend, root.path());
        match failure {
            Review25RetirementFailure::Transition => {
                backend.fail_artifact_registry_removals_for_test()
            }
            Review25RetirementFailure::Unlink => {
                backend.fail_artifact_registry_retirement_unlink_for_test()
            }
            Review25RetirementFailure::FinalSync => {
                backend.fail_artifact_registry_retirement_final_sync_for_test()
            }
        }

        backend
            .release_cleanup_recovery_path(&path)
            .expect_err("a durable retirement failure must remain visible");
        assert!(backend.cleanup_recovery_anchor(&path).is_some());

        backend.clear_artifact_registry_retirement_failures_for_test();
        backend.release_cleanup_recovery_path(&path).unwrap();
        assert_eq!(artifact_registry_record_count(&backend), 0);
        drop(backend);

        let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
        assert!(!restarted.startup_recovery_paths_for_test().contains(&path));
        assert_eq!(artifact_registry_record_count(&restarted), 0);
        drop(restarted);

        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert!(!second_restart
            .startup_recovery_paths_for_test()
            .contains(&path));
        assert_eq!(artifact_registry_record_count(&second_restart), 0);
    }
}

#[cfg(unix)]
#[test]
fn review25_legacy_artifact_record_is_cas_migrated_before_authorization() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let path = PathBuf::from("/.owned.zaplex-transfer-review25-legacy");
    fs::write(
        root.path().join(".owned.zaplex-transfer-review25-legacy"),
        b"owned",
    )
    .unwrap();
    let anchor = backend
        .existing_entry_ownership_anchor(&path)
        .unwrap()
        .unwrap();
    backend.persist_artifact_identity(&path, anchor).unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let active = backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap();
    let legacy_record_payload = legacy_payload(
        &registry.artifact_payload(&active),
        LEGACY_TRANSFER_ARTIFACT_REGISTRY_VERSION,
    );
    let legacy_contents = registry.signed_payload(&legacy_record_payload);
    let record_path = registry.artifact_record_path(&path);
    fs::write(&record_path, &legacy_contents).unwrap();
    let stale_legacy = registry
        .read_artifact_record_unlocked(&record_path)
        .unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let migrated = restarted
        .persistent_artifact_records
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .expect("authenticated legacy recovery must remain reachable");
    assert!(!migrated.generation.starts_with("legacy-"));
    let migrated_contents = fs::read_to_string(&record_path).unwrap();
    assert!(migrated_contents.starts_with(TRANSFER_ARTIFACT_REGISTRY_VERSION));
    assert_ne!(migrated_contents, legacy_contents);

    let registry = restarted.directory_reservation_registry.as_ref().unwrap();
    assert!(
        registry.write_artifact_record(&stale_legacy).is_err(),
        "an old writer must not overwrite the migrated generation"
    );
    assert_eq!(fs::read_to_string(&record_path).unwrap(), migrated_contents);
}

#[cfg(unix)]
#[test]
fn review25_failed_legacy_artifact_migration_stays_unresolved_without_authority() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let path = PathBuf::from("/.owned.zaplex-transfer-review25-migration-failure");
    fs::write(
        root.path()
            .join(".owned.zaplex-transfer-review25-migration-failure"),
        b"owned",
    )
    .unwrap();
    let anchor = backend
        .existing_entry_ownership_anchor(&path)
        .unwrap()
        .unwrap();
    backend.persist_artifact_identity(&path, anchor).unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let active = backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap();
    let legacy_record_payload = legacy_payload(
        &registry.artifact_payload(&active),
        LEGACY_TRANSFER_ARTIFACT_REGISTRY_VERSION,
    );
    let record_path = registry.artifact_record_path(&path);
    fs::write(
        &record_path,
        registry.signed_payload(&legacy_record_payload),
    )
    .unwrap();
    backend.persistent_artifact_records.lock().unwrap().clear();
    backend.cleanup_recovery_identities.lock().unwrap().clear();
    backend.startup_unresolved_paths.lock().unwrap().clear();
    registry
        .fail_artifact_legacy_migration
        .store(true, Ordering::SeqCst);

    backend.discover_persistent_artifacts();

    assert!(!backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .contains_key(&path));
    assert!(backend.cleanup_recovery_anchor(&path).is_none());
    assert!(
        !backend.startup_unresolved_paths.lock().unwrap().is_empty(),
        "migration failure must remain globally visible without cleanup authority"
    );
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let migrated = restarted
        .persistent_artifact_records
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .expect("a later restart must safely migrate the authenticated legacy record");
    assert!(!migrated.legacy);
    assert!(!migrated.generation.starts_with("legacy-"));
}

#[cfg(unix)]
#[test]
fn review25_legacy_namespace_record_is_cas_migrated_before_authorization() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let namespace = backend
        .directory_reservation_namespaces
        .lock()
        .unwrap()
        .values()
        .next()
        .cloned()
        .unwrap();
    let record_path = registry.record_path(namespace.device);
    let record = registry.read_namespace_record(&record_path).unwrap();
    let legacy_record_payload = legacy_payload(
        &registry.namespace_payload(
            &record.path,
            record.device,
            &record.namespace_id,
            &record.object_id,
            &record.generation,
        ),
        LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
    );
    let legacy_contents = registry.signed_payload(&legacy_record_payload);
    fs::write(&record_path, &legacy_contents).unwrap();
    let marker_path = record.path.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
    let marker_payload = registry
        .verify_marker(&fs::read_to_string(&marker_path).unwrap())
        .unwrap();
    let legacy_marker = registry.signed_payload(&legacy_payload(
        &marker_payload,
        LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
    ));
    fs::write(&marker_path, legacy_marker).unwrap();
    let stale_legacy = registry
        .read_namespace_record_unlocked(&record_path)
        .unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let migrated_contents = fs::read_to_string(&record_path).unwrap();
    assert!(migrated_contents.starts_with(DIRECTORY_RESERVATION_REGISTRY_VERSION));
    assert_ne!(migrated_contents, legacy_contents);
    let migrated = restarted
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .read_namespace_record(&record_path)
        .unwrap();
    assert!(!migrated.generation.starts_with("legacy-"));
    assert_eq!(
        restarted.directory_reservation_namespace_path_for_test(),
        Some(record.path)
    );
    let registry = restarted.directory_reservation_registry.as_ref().unwrap();
    assert!(registry.write_namespace_record(&stale_legacy).is_err());
    assert_eq!(fs::read_to_string(&record_path).unwrap(), migrated_contents);
}

#[cfg(unix)]
#[test]
fn review25_namespace_migration_crash_cutpoints_converge_without_authority_gap() {
    for after_marker_replace in [true, false] {
        let root = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf())
            .with_in_tree_directory_reservation_namespace();
        backend
            .create_dir_with_ownership_anchor(Path::new("/stage"))
            .unwrap();
        let registry = backend.directory_reservation_registry.as_ref().unwrap();
        let namespace = backend
            .directory_reservation_namespaces
            .lock()
            .unwrap()
            .values()
            .next()
            .cloned()
            .unwrap();
        let record_path = registry.record_path(namespace.device);
        let record = registry.read_namespace_record(&record_path).unwrap();
        let legacy_record_payload = legacy_payload(
            &registry.namespace_payload(
                &record.path,
                record.device,
                &record.namespace_id,
                &record.object_id,
                &record.generation,
            ),
            LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
        );
        fs::write(
            &record_path,
            registry.signed_payload(&legacy_record_payload),
        )
        .unwrap();
        let marker_path = record.path.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
        let marker_payload = registry
            .verify_marker(&fs::read_to_string(&marker_path).unwrap())
            .unwrap();
        fs::write(
            &marker_path,
            registry.signed_payload(&legacy_payload(
                &marker_payload,
                LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
            )),
        )
        .unwrap();
        backend
            .directory_reservation_namespaces
            .lock()
            .unwrap()
            .clear();
        backend.startup_unresolved_paths.lock().unwrap().clear();
        if after_marker_replace {
            registry
                .fail_namespace_migration_after_marker_replace
                .store(true, Ordering::SeqCst);
        } else {
            registry
                .fail_namespace_migration_after_record_replace
                .store(true, Ordering::SeqCst);
        }

        backend.discover_root_directory_reservations();

        assert!(
            backend
                .directory_reservation_namespaces
                .lock()
                .unwrap()
                .is_empty(),
            "a failed migration phase must not grant namespace authority"
        );
        assert!(
            !backend.startup_unresolved_paths.lock().unwrap().is_empty(),
            "a failed migration phase must remain durably visible"
        );
        drop(backend);

        let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(
            restarted.directory_reservation_namespace_path_for_test(),
            Some(record.path.clone())
        );
        let registry = restarted.directory_reservation_registry.as_ref().unwrap();
        let migrated = registry.read_namespace_record(&record_path).unwrap();
        assert!(!migrated.legacy);
        let marker_payload = registry
            .verify_marker(&fs::read_to_string(&marker_path).unwrap())
            .unwrap();
        assert_eq!(
            marker_payload,
            registry.namespace_payload(
                &migrated.path,
                migrated.device,
                &migrated.namespace_id,
                &migrated.object_id,
                &migrated.generation,
            )
        );
        drop(restarted);

        let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
        assert_eq!(
            second_restart.directory_reservation_namespace_path_for_test(),
            Some(record.path)
        );
        assert!(second_restart.startup_recovery_paths_for_test().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn review25_namespace_record_temporaries_are_bounded_on_every_exit() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    backend
        .create_dir_with_ownership_anchor(Path::new("/first-stage"))
        .unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let record_path = fs::read_dir(&registry.root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("namespace-") && name.ends_with(".record")
        })
        .unwrap()
        .path();
    let record = registry.read_namespace_record(&record_path).unwrap();

    for failure in [
        NamespaceProbeFailure::Record,
        NamespaceProbeFailure::NamespacePath,
    ] {
        *registry.namespace_probe_failure.lock().unwrap() = Some(failure);
        for _ in 0..4 {
            let mut conflicting = record.clone();
            conflicting.generation = uuid::Uuid::new_v4().to_string();
            assert!(registry.write_namespace_record(&conflicting).is_err());
        }
        assert_eq!(
            namespace_temporary_record_count(&backend),
            0,
            "failed namespace probes must not accumulate temporary records"
        );
    }

    *registry.namespace_probe_failure.lock().unwrap() = None;
    registry.write_namespace_record(&record).unwrap();
    assert_eq!(namespace_temporary_record_count(&backend), 0);

    let mut conflicting = record.clone();
    conflicting.generation = uuid::Uuid::new_v4().to_string();
    assert!(registry.write_namespace_record(&conflicting).is_err());
    assert_eq!(namespace_temporary_record_count(&backend), 0);

    *registry.namespace_probe_failure.lock().unwrap() = Some(NamespaceProbeFailure::Parent);
    assert!(backend
        .create_dir_with_ownership_anchor(Path::new("/parent-probe-stage"))
        .is_err());
    assert_eq!(namespace_temporary_record_count(&backend), 0);
}

fn assert_review26_association_cutpoint_is_lifecycle_locked(
    backend: &InMemorySftpBackend,
    _path: &Path,
) {
    assert!(
        backend.artifact_lifecycle.try_lock().is_err(),
        "artifact retirement must not enter between durable record and association publication"
    );
}

#[cfg(unix)]
#[test]
fn review26_exchange_resolution_publishes_anchor_in_one_lifecycle_transaction() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_artifact_association_cutpoint(
            assert_review26_association_cutpoint_is_lifecycle_locked,
        );
    let owned = root.path().join("owned-private");
    fs::write(&owned, b"owned").unwrap();
    let owned_identity =
        stable_identity_from_local_metadata(&fs::symlink_metadata(&owned).unwrap());
    let displaced = root.path().join("displaced-identity");
    fs::write(&displaced, b"displaced").unwrap();
    let displaced_identity =
        stable_identity_from_local_metadata(&fs::symlink_metadata(&displaced).unwrap());
    fs::remove_file(&displaced).unwrap();
    let record = PersistentExchangeRecord {
        path: PathBuf::from("/.review26-exchange-resolution"),
        first: PersistentExchangeCandidate {
            physical_path: root.path().join("absent-public"),
            role: "owned".to_string(),
            identity: Some(owned_identity),
        },
        second: PersistentExchangeCandidate {
            physical_path: owned,
            role: "displaced".to_string(),
            identity: Some(displaced_identity),
        },
        phase: PersistentExchangePhase::Applied,
        generation: uuid::Uuid::new_v4().to_string(),
        legacy: false,
    };
    backend.persist_exchange_record(record.clone()).unwrap();

    backend.resolve_persistent_exchange(&record).unwrap();
}

#[cfg(unix)]
#[test]
fn review26_directory_recovery_publishes_all_associations_in_one_transaction() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_artifact_association_cutpoint(
            assert_review26_association_cutpoint_is_lifecycle_locked,
        );
    let private = root.path().join("private-directory");
    fs::create_dir(&private).unwrap();
    let file = open_local_cleanup_anchor(&private).unwrap();
    let anchor: Arc<dyn BackendOwnershipAnchor> = Arc::new(LocalOwnershipAnchor {
        file,
        root: backend.root.clone(),
        opaque_paths: Some(backend.opaque_recovery_paths.clone()),
    });
    let identity = anchor.identity().unwrap();

    backend
        .register_directory_reservation_recovery(&private, None, Some(identity), Some(anchor))
        .unwrap();
}

#[cfg(unix)]
#[test]
fn review26_sibling_resolution_publishes_anchor_in_one_lifecycle_transaction() {
    let root = tempdir().unwrap();
    let sibling_root = tempdir().unwrap();
    let physical = sibling_root.path().join("moved-owned");
    fs::write(&physical, b"owned").unwrap();
    let expected = stable_identity_from_local_metadata(&fs::symlink_metadata(&physical).unwrap());
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_artifact_association_cutpoint(
            assert_review26_association_cutpoint_is_lifecycle_locked,
        );
    let logical = PathBuf::from("/.review26-sibling-rescan");
    let record = PersistentArtifactRecord::active(
        logical.clone(),
        Some(sibling_root.path().to_path_buf()),
        "rescan-anchor-sibling:owned".to_string(),
        Some(expected),
    );
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry.write_artifact_record(&record).unwrap();
    backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .insert(logical.clone(), record);

    backend.retry_unresolved_recovery(&logical).unwrap();
}

#[cfg(unix)]
#[test]
fn review26_exchange_transition_publishes_displaced_anchor_atomically() {
    let root = tempdir().unwrap();
    fs::write(root.path().join(".source.zaplex-transfer-1"), b"source").unwrap();
    fs::write(root.path().join("target"), b"target").unwrap();
    let hook_called = Arc::new(AtomicBool::new(false));
    let hook_called_for_cutpoint = hook_called.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_artifact_association_cutpoint(move |backend, path| {
            hook_called_for_cutpoint.store(true, Ordering::SeqCst);
            assert_review26_association_cutpoint_is_lifecycle_locked(backend, path);
        });
    let source = Path::new("/.source.zaplex-transfer-1");
    let source_anchor = backend
        .existing_entry_ownership_anchor(source)
        .unwrap()
        .unwrap();
    backend
        .persist_artifact_identity(source, source_anchor)
        .unwrap();

    backend.replace(source, Path::new("/target")).unwrap();
    assert!(hook_called.load(Ordering::SeqCst));
}

fn review26_exchange_record(root: &Path) -> PersistentExchangeRecord {
    PersistentExchangeRecord {
        path: PathBuf::from("/.review26-exchange-retirement"),
        first: PersistentExchangeCandidate {
            physical_path: root.join("first"),
            role: "first".to_string(),
            identity: None,
        },
        second: PersistentExchangeCandidate {
            physical_path: root.join("second"),
            role: "second".to_string(),
            identity: None,
        },
        phase: PersistentExchangePhase::Prepared,
        generation: uuid::Uuid::new_v4().to_string(),
        legacy: false,
    }
}

#[cfg(unix)]
#[test]
fn review26_exchange_unlink_failure_preserves_same_process_authority() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let record = review26_exchange_record(root.path());
    backend.persist_exchange_record(record.clone()).unwrap();
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .fail_exchange_retirement_unlink
        .store(true, Ordering::SeqCst);

    backend
        .release_exchange_record(&record.path)
        .expect_err("injected unlink failure must be visible");

    assert!(backend
        .persistent_exchange_records
        .lock()
        .unwrap()
        .contains_key(&record.path));
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .fail_exchange_retirement_unlink
        .store(false, Ordering::SeqCst);
    backend.release_exchange_record(&record.path).unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!restarted
        .startup_recovery_paths_for_test()
        .contains(&record.path));
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!second_restart
        .startup_recovery_paths_for_test()
        .contains(&record.path));
}

#[cfg(unix)]
#[test]
fn review26_exchange_final_fsync_failure_preserves_same_process_authority() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let record = review26_exchange_record(root.path());
    backend.persist_exchange_record(record.clone()).unwrap();
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .fail_exchange_retirement_final_sync
        .store(true, Ordering::SeqCst);

    backend
        .release_exchange_record(&record.path)
        .expect_err("injected final fsync failure must be visible");

    assert!(backend
        .persistent_exchange_records
        .lock()
        .unwrap()
        .contains_key(&record.path));
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .fail_exchange_retirement_final_sync
        .store(false, Ordering::SeqCst);
    backend.release_exchange_record(&record.path).unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!restarted
        .startup_recovery_paths_for_test()
        .contains(&record.path));
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(!second_restart
        .startup_recovery_paths_for_test()
        .contains(&record.path));
}

fn review26_registry_temporary_count(registry: &DirectoryReservationRegistry) -> usize {
    fs::read_dir(&registry.root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count()
}

#[cfg(unix)]
#[test]
fn review26_namespace_cleanup_failures_keep_temporary_files_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap();
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let record_path = fs::read_dir(&registry.root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("namespace-")
        })
        .unwrap()
        .path();
    let record = registry.read_namespace_record(&record_path).unwrap();
    registry
        .fail_temporary_cleanup
        .store(true, Ordering::SeqCst);
    *registry.namespace_probe_failure.lock().unwrap() = Some(NamespaceProbeFailure::Record);

    for _ in 0..4 {
        assert!(registry.write_namespace_record(&record).is_err());
    }

    assert!(review26_registry_temporary_count(registry) <= 1);
    let foreign = registry.root.join(".namespace-foreign.tmp");
    fs::write(&foreign, b"foreign").unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        review26_registry_temporary_count(
            restarted.directory_reservation_registry.as_ref().unwrap()
        ),
        1,
        "restart must prune only exact owned temporary slots"
    );
    assert_eq!(fs::read(foreign).unwrap(), b"foreign");
}

#[cfg(unix)]
#[test]
fn review26_exchange_write_failures_keep_temporary_files_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry
        .fail_exchange_temporary_write
        .store(true, Ordering::SeqCst);
    let record = review26_exchange_record(root.path());

    for _ in 0..4 {
        assert!(registry.write_exchange_record(&record).is_err());
    }

    assert!(review26_registry_temporary_count(registry) <= 1);
    drop(backend);
    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        review26_registry_temporary_count(
            restarted.directory_reservation_registry.as_ref().unwrap()
        ),
        0
    );
}

#[cfg(unix)]
#[test]
fn review26_artifact_write_failures_keep_temporary_files_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry
        .fail_artifact_temporary_write
        .store(true, Ordering::SeqCst);
    let record = PersistentArtifactRecord::active(
        PathBuf::from("/.review26-artifact-temp"),
        None,
        "unresolved-review26".to_string(),
        None,
    );

    for _ in 0..4 {
        assert!(registry.write_artifact_record(&record).is_err());
    }

    assert!(review26_registry_temporary_count(registry) <= 1);
    drop(backend);
    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        review26_registry_temporary_count(
            restarted.directory_reservation_registry.as_ref().unwrap()
        ),
        0
    );
}

fn prepare_review26_legacy_namespace(
    backend: &InMemorySftpBackend,
) -> (PathBuf, PathBuf, DirectoryNamespaceRecord) {
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let namespace = backend
        .directory_reservation_namespaces
        .lock()
        .unwrap()
        .values()
        .next()
        .cloned()
        .unwrap();
    let record_path = registry.record_path(namespace.device);
    let record = registry.read_namespace_record(&record_path).unwrap();
    let legacy_record = legacy_payload(
        &registry.namespace_payload(
            &record.path,
            record.device,
            &record.namespace_id,
            &record.object_id,
            &record.generation,
        ),
        LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
    );
    fs::write(&record_path, registry.signed_payload(&legacy_record)).unwrap();
    let marker_path = record.path.join(DIRECTORY_RESERVATION_NAMESPACE_MARKER);
    let marker_payload = registry
        .verify_marker(&fs::read_to_string(&marker_path).unwrap())
        .unwrap();
    fs::write(
        &marker_path,
        registry.signed_payload(&legacy_payload(
            &marker_payload,
            LEGACY_DIRECTORY_RESERVATION_REGISTRY_VERSION,
        )),
    )
    .unwrap();
    (record_path, marker_path, record)
}

#[cfg(unix)]
#[test]
fn review26_namespace_migration_marker_cleanup_failure_is_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap();
    let (record_path, _, record) = prepare_review26_legacy_namespace(&backend);
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry
        .fail_temporary_cleanup
        .store(true, Ordering::SeqCst);
    registry
        .fail_namespace_migration_marker_temporary_write
        .store(true, Ordering::SeqCst);

    for _ in 0..4 {
        assert!(registry.read_namespace_record(&record_path).is_err());
    }

    let count = fs::read_dir(&record.path)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert!(count <= 1);
    drop(backend);
    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    let count = fs::read_dir(&record.path)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(count, 0);
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        second_restart.directory_reservation_namespace_path_for_test(),
        Some(record.path)
    );
}

#[cfg(unix)]
#[test]
fn review26_namespace_migration_record_cleanup_failure_is_bounded() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_in_tree_directory_reservation_namespace();
    backend
        .create_dir_with_ownership_anchor(Path::new("/stage"))
        .unwrap();
    let (record_path, _, _) = prepare_review26_legacy_namespace(&backend);
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry
        .fail_temporary_cleanup
        .store(true, Ordering::SeqCst);
    registry
        .fail_namespace_migration_record_temporary_write
        .store(true, Ordering::SeqCst);

    for _ in 0..4 {
        assert!(registry.read_namespace_record(&record_path).is_err());
    }

    assert!(review26_registry_temporary_count(registry) <= 1);
    drop(backend);
    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        review26_registry_temporary_count(
            restarted.directory_reservation_registry.as_ref().unwrap()
        ),
        0
    );
    drop(restarted);
    let second_restart = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(second_restart
        .directory_reservation_namespace_path_for_test()
        .is_some());
}

fn assert_review27_lifecycle_publication_is_locked(backend: &InMemorySftpBackend, _path: &Path) {
    assert!(
        backend.artifact_lifecycle.try_lock().is_err(),
        "retirement must not enter between durable publication and in-memory associations"
    );
}

#[cfg(unix)]
#[test]
fn review27_owned_failed_directory_candidate_is_published_atomically() {
    let root = tempdir().unwrap();
    let private = root.path().join("owned-directory-candidate");
    fs::create_dir(&private).unwrap();
    let expected = stable_identity_from_local_metadata(&fs::symlink_metadata(&private).unwrap());
    let executed = Arc::new(AtomicBool::new(false));
    let executed_at_cutpoint = executed.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_failed_directory_candidate_association_cutpoint(move |backend, path, owned| {
            assert!(owned);
            executed_at_cutpoint.store(true, Ordering::SeqCst);
            assert_review27_lifecycle_publication_is_locked(backend, path);
        });

    let paths = backend
        .register_failed_directory_reservation_candidates(&private, Some(&expected))
        .unwrap();

    assert!(executed.load(Ordering::SeqCst));
    assert_eq!(paths.len(), 1);
    assert!(backend
        .cleanup_recovery_identities
        .lock()
        .unwrap()
        .contains_key(&paths[0]));
}

#[cfg(unix)]
#[test]
fn review27_ambiguous_failed_directory_candidate_is_published_atomically() {
    let root = tempdir().unwrap();
    let private = root.path().join("absent-directory-candidate");
    let executed = Arc::new(AtomicBool::new(false));
    let executed_at_cutpoint = executed.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_failed_directory_candidate_association_cutpoint(move |backend, path, owned| {
            assert!(!owned);
            executed_at_cutpoint.store(true, Ordering::SeqCst);
            assert_review27_lifecycle_publication_is_locked(backend, path);
        });

    let paths = backend
        .register_failed_directory_reservation_candidates(&private, None)
        .unwrap();

    assert!(executed.load(Ordering::SeqCst));
    assert_eq!(paths.len(), 1);
    assert!(backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .contains_key(&paths[0]));
}

#[cfg(unix)]
#[test]
fn review27_opaque_cleanup_sibling_is_durable_before_mapping() {
    let root = tempdir().unwrap();
    let logical = PathBuf::from("/.source.zaplex-source-review27");
    let sibling = PathBuf::from("/.source.zaplex-delete-review27");
    let physical = root.path().join("private-source");
    fs::write(&physical, b"owned").unwrap();
    let executed = Arc::new(AtomicBool::new(false));
    let executed_at_cutpoint = executed.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_opaque_cleanup_sibling_publication_cutpoint(move |backend, path| {
            executed_at_cutpoint.store(true, Ordering::SeqCst);
            assert_review27_lifecycle_publication_is_locked(backend, path);
        });
    let source_record = PersistentArtifactRecord::active(
        logical.clone(),
        Some(physical.clone()),
        "source-review27".to_string(),
        None,
    );
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .write_artifact_record(&source_record)
        .unwrap();
    backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .insert(logical.clone(), source_record);
    backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .insert(logical.clone(), physical);

    backend
        .map_opaque_cleanup_sibling(&logical, &sibling)
        .unwrap();

    assert!(executed.load(Ordering::SeqCst));
    assert!(backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .contains_key(&sibling));
    assert!(backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .contains_key(&sibling));
}

fn review27_exchange_record(root: &Path, suffix: &str) -> PersistentExchangeRecord {
    PersistentExchangeRecord::active(
        PathBuf::from(format!("/.review27-exchange-{suffix}")),
        PersistentExchangeCandidate {
            physical_path: root.join(format!("{suffix}-first")),
            role: "first".to_string(),
            identity: None,
        },
        PersistentExchangeCandidate {
            physical_path: root.join(format!("{suffix}-second")),
            role: "second".to_string(),
            identity: None,
        },
        PersistentExchangePhase::Prepared,
    )
}

#[cfg(unix)]
#[test]
fn review27_exchange_create_probe_errors_preserve_existing_generation() {
    for errno in [libc::EACCES, libc::EIO] {
        let root = tempdir().unwrap();
        let backend = InMemorySftpBackend::new(root.path().to_path_buf());
        let registry = backend.directory_reservation_registry.as_ref().unwrap();
        let original = review27_exchange_record(root.path(), &format!("probe-{errno}"));
        registry.write_exchange_record(&original).unwrap();
        let record_path = registry.exchange_record_path(&original.path);
        let original_bytes = fs::read(&record_path).unwrap();
        *registry.exchange_create_probe_error.lock().unwrap() = Some(errno);
        let mut replacement = original.clone();
        replacement.generation = uuid::Uuid::new_v4().to_string();

        assert!(registry.write_exchange_record(&replacement).is_err());
        assert_eq!(fs::read(&record_path).unwrap(), original_bytes);
        *registry.exchange_create_probe_error.lock().unwrap() = None;
        let retained = registry.read_exchange_record(&record_path).unwrap();
        assert!(same_persistent_exchange_record(&retained, &original));
    }
}

#[cfg(unix)]
#[test]
fn review27_exchange_create_is_idempotent_only_for_the_exact_generation() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let original = review27_exchange_record(root.path(), "idempotent");
    backend.persist_exchange_record(original.clone()).unwrap();
    backend.persist_exchange_record(original.clone()).unwrap();
    let mut replacement = original.clone();
    replacement.generation = uuid::Uuid::new_v4().to_string();
    assert!(backend.persist_exchange_record(replacement).is_err());
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    restarted.persist_exchange_record(original.clone()).unwrap();
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&original.path));
}

#[cfg(unix)]
#[test]
fn review28_exact_exchange_retry_requires_successful_directory_fsync() {
    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    let record = review27_exchange_record(root.path(), "review28-fsync");
    registry
        .fail_exchange_create_sync
        .store(true, Ordering::SeqCst);

    assert!(
        backend.persist_exchange_record(record.clone()).is_err(),
        "the first create must report the failed registry-directory fsync"
    );
    assert!(registry.exchange_record_path(&record.path).is_file());
    assert!(
        backend.persist_exchange_record(record.clone()).is_err(),
        "an exact visible record must not acknowledge durability while fsync still fails"
    );
    let mut different_generation = record.clone();
    different_generation.generation = uuid::Uuid::new_v4().to_string();
    assert!(backend
        .persist_exchange_record(different_generation)
        .is_err());

    registry
        .fail_exchange_create_sync
        .store(false, Ordering::SeqCst);
    backend.persist_exchange_record(record.clone()).unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted
        .startup_recovery_paths_for_test()
        .contains(&record.path));
    let restarted_registry = restarted.directory_reservation_registry.as_ref().unwrap();
    let retained = restarted_registry
        .read_exchange_record(&restarted_registry.exchange_record_path(&record.path))
        .unwrap();
    assert!(same_persistent_exchange_record(&retained, &record));
    drop(restarted);

    let restarted_again = InMemorySftpBackend::new(root.path().to_path_buf());
    assert!(restarted_again
        .startup_recovery_paths_for_test()
        .contains(&record.path));
    let restarted_registry = restarted_again
        .directory_reservation_registry
        .as_ref()
        .unwrap();
    let retained = restarted_registry
        .read_exchange_record(&restarted_registry.exchange_record_path(&record.path))
        .unwrap();
    assert!(same_persistent_exchange_record(&retained, &record));
}

#[cfg(unix)]
#[test]
fn review28_opaque_sibling_cannot_publish_from_a_stale_source_generation() {
    let root = tempdir().unwrap();
    let first_parent = root.path().join("generation-one");
    let second_parent = root.path().join("generation-two");
    fs::create_dir(&first_parent).unwrap();
    fs::create_dir(&second_parent).unwrap();
    let first_physical = first_parent.join("source");
    let second_physical = second_parent.join("source");
    fs::write(&first_physical, b"first").unwrap();
    fs::write(&second_physical, b"second").unwrap();
    let logical = PathBuf::from("/.source.zaplex-source-review28");
    let sibling = PathBuf::from("/.source.zaplex-delete-review28");
    let cutpoint_executed = Arc::new(AtomicBool::new(false));
    let cutpoint_mutated = Arc::new(AtomicBool::new(false));
    let executed = cutpoint_executed.clone();
    let mutated = cutpoint_mutated.clone();
    let replacement_physical = second_physical.clone();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf())
        .with_after_opaque_cleanup_source_read_before_lifecycle(move |backend, path| {
            executed.store(true, Ordering::SeqCst);
            let Ok(_lifecycle) = backend.artifact_lifecycle.try_lock() else {
                return;
            };
            let current = backend
                .persistent_artifact_records
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .unwrap();
            let mut replacement = current.clone();
            replacement.generation = uuid::Uuid::new_v4().to_string();
            replacement.physical_path = Some(replacement_physical.clone());
            backend
                .directory_reservation_registry
                .as_ref()
                .unwrap()
                .transition_artifact_record(&current, &replacement)
                .unwrap();
            backend
                .persistent_artifact_records
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), replacement);
            backend
                .opaque_recovery_paths
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), replacement_physical.clone());
            mutated.store(true, Ordering::SeqCst);
        });
    let source_record = PersistentArtifactRecord::active(
        logical.clone(),
        Some(first_physical.clone()),
        "source-review28".to_string(),
        None,
    );
    backend
        .directory_reservation_registry
        .as_ref()
        .unwrap()
        .write_artifact_record(&source_record)
        .unwrap();
    backend
        .persistent_artifact_records
        .lock()
        .unwrap()
        .insert(logical.clone(), source_record);
    backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .insert(logical.clone(), first_physical);

    backend
        .map_opaque_cleanup_sibling(&logical, &sibling)
        .unwrap();

    assert!(cutpoint_executed.load(Ordering::SeqCst));
    assert!(
        !cutpoint_mutated.load(Ordering::SeqCst),
        "the source generation must remain protected throughout sibling publication"
    );
    let current_source = backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .get(&logical)
        .cloned()
        .unwrap();
    let sibling_physical = backend
        .opaque_recovery_paths
        .lock()
        .unwrap()
        .get(&sibling)
        .cloned()
        .unwrap();
    assert_eq!(sibling_physical.parent(), current_source.parent());
}

#[cfg(unix)]
#[test]
fn review27_distinct_failed_writes_use_a_global_bounded_temp_pool() {
    const MAX_REGISTRY_ROOT_TEMPORARIES: usize = 4;

    let root = tempdir().unwrap();
    let backend = InMemorySftpBackend::new(root.path().to_path_buf());
    let registry = backend.directory_reservation_registry.as_ref().unwrap();
    registry
        .fail_temporary_cleanup
        .store(true, Ordering::SeqCst);
    registry
        .fail_exchange_temporary_write
        .store(true, Ordering::SeqCst);
    registry
        .fail_artifact_temporary_write
        .store(true, Ordering::SeqCst);

    for index in 0..32 {
        let exchange = review27_exchange_record(root.path(), &format!("temp-{index}"));
        assert!(registry.write_exchange_record(&exchange).is_err());
        let artifact = PersistentArtifactRecord::active(
            PathBuf::from(format!("/.review27-artifact-temp-{index}")),
            None,
            "unresolved-review27".to_string(),
            None,
        );
        assert!(registry.write_artifact_record(&artifact).is_err());
    }

    assert!(
        review26_registry_temporary_count(registry) <= MAX_REGISTRY_ROOT_TEMPORARIES,
        "distinct failed writes must share a fixed registry-wide temporary pool"
    );
    let committed = registry.root.join("committed-not-temporary.record");
    let foreign = registry.root.join(".foreign-review27.tmp");
    fs::write(&committed, b"committed").unwrap();
    fs::write(&foreign, b"foreign").unwrap();
    drop(backend);

    let restarted = InMemorySftpBackend::new(root.path().to_path_buf());
    assert_eq!(
        review26_registry_temporary_count(
            restarted.directory_reservation_registry.as_ref().unwrap()
        ),
        1
    );
    assert_eq!(fs::read(committed).unwrap(), b"committed");
    assert_eq!(fs::read(foreign).unwrap(), b"foreign");
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum LiveRenameFault {
    BeforeMutation,
    AfterMutation,
}

#[cfg(unix)]
fn live_sftp_configuration() -> (String, u16, String, PathBuf, PathBuf) {
    let host = std::env::var("ZAPLEX_LIVE_SFTP_HOST")
        .expect("ZAPLEX_LIVE_SFTP_HOST is required for the ignored live SFTP tests");
    let port = std::env::var("ZAPLEX_LIVE_SFTP_PORT")
        .expect("ZAPLEX_LIVE_SFTP_PORT is required for the ignored live SFTP tests")
        .parse()
        .expect("ZAPLEX_LIVE_SFTP_PORT must be a valid u16");
    let username = std::env::var("ZAPLEX_LIVE_SFTP_USERNAME")
        .expect("ZAPLEX_LIVE_SFTP_USERNAME is required for the ignored live SFTP tests");
    let key_path = PathBuf::from(
        std::env::var("ZAPLEX_LIVE_SFTP_KEY_PATH")
            .expect("ZAPLEX_LIVE_SFTP_KEY_PATH is required for the ignored live SFTP tests"),
    );
    let root = PathBuf::from(
        std::env::var("ZAPLEX_LIVE_SFTP_ROOT")
            .expect("ZAPLEX_LIVE_SFTP_ROOT is required for the ignored live SFTP tests"),
    );
    (host, port, username, key_path, root)
}

#[cfg(unix)]
fn spawn_live_safe_file_client(
    journal_path: PathBuf,
    fault: Option<LiveRenameFault>,
) -> (
    Arc<remote_server::client::RemoteServerClient>,
    warpui::r#async::executor::Background,
    tokio::task::JoinHandle<()>,
) {
    use remote_server::proto::{client_message, safe_file_request, server_message, ServerMessage};
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let connection_id = uuid::Uuid::new_v4();
    let server_task = tokio::spawn(async move {
        let mut reader = server_read.compat();
        let mut writer = server_write.compat_write();
        let mut server =
            crate::remote_server::safe_file::SafeFileServer::new_for_test(journal_path);
        loop {
            let message = match remote_server::protocol::read_client_message(&mut reader).await {
                Ok(message) => message,
                Err(remote_server::protocol::ProtocolError::UnexpectedEof) => break,
                Err(error) => panic!("live safe-file test transport failed: {error}"),
            };
            let Some(client_message::Message::SafeFile(request)) = message.message else {
                panic!("live safe-file test received an unexpected request");
            };
            let is_rename = matches!(
                request.operation.as_ref(),
                Some(safe_file_request::Operation::Rename(_))
            );
            if is_rename && matches!(fault, Some(LiveRenameFault::BeforeMutation)) {
                break;
            }
            let response = server.handle(connection_id, request);
            if is_rename && matches!(fault, Some(LiveRenameFault::AfterMutation)) {
                break;
            }
            remote_server::protocol::write_server_message(
                &mut writer,
                &ServerMessage {
                    request_id: message.request_id,
                    message: Some(server_message::Message::SafeFileResponse(response)),
                },
            )
            .await
            .expect("live safe-file test response should be writable");
        }
        server.close_connection(connection_id);
    });

    let executor = warpui::r#async::executor::Background::default();
    let (client, _events) = remote_server::client::RemoteServerClient::new(
        client_read.compat(),
        client_write.compat_write(),
        &executor,
    );
    (Arc::new(client), executor, server_task)
}

#[cfg(unix)]
async fn verify_live_sftp_rename_recovery(fault: LiveRenameFault) {
    let (host, port, username, key_path, root) = live_sftp_configuration();
    let case_root = root.join(format!("zaplex-live-sftp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&case_root).unwrap();
    let staged = case_root.join(".payload.zaplex-transfer-live");
    let destination = case_root.join("payload.bin");
    let local = tempdir().unwrap();
    let local_source = local.path().join("source.bin");
    fs::write(&local_source, b"live-sftp-payload").unwrap();

    let session = zap_sftp::SftpSession::connect(
        &host,
        port,
        &username,
        zap_sftp::AuthMethod::PublicKey {
            key_path,
            passphrase: None,
        },
        Some(std::time::Duration::from_secs(10)),
    )
    .expect("the CI OpenSSH fixture must accept the configured key");
    let sftp = session.sftp().expect("the live SFTP subsystem must open");
    let journal = tempdir().unwrap();
    let slot = SafeFileClientSlot::default();
    let (first_client, _first_executor, first_server) =
        spawn_live_safe_file_client(journal.path().to_path_buf(), Some(fault));
    slot.set(Some(first_client));
    let backend = LiveSftpBackend::new_with_safe_file_slot(sftp, slot.clone());

    let mut writer = backend
        .create_file_writer(&staged)
        .expect("the daemon must exclusively create the remote staging file");
    writer.write_chunk(b"live-sftp-payload").unwrap();
    writer.flush().unwrap();
    let anchor = writer
        .ownership_anchor()
        .unwrap()
        .expect("the live writer must retain an immutable daemon handle");
    let error = backend
        .rename_if_matches(&staged, &destination, anchor)
        .expect_err("the injected transport loss must require recovery");
    assert!(matches!(error, SftpOpsError::RecoveryRequired { .. }));
    assert_eq!(fs::read(&local_source).unwrap(), b"live-sftp-payload");
    drop(writer);
    first_server.await.unwrap();

    let (second_client, _second_executor, second_server) =
        spawn_live_safe_file_client(journal.path().to_path_buf(), None);
    slot.set(Some(second_client));
    assert_eq!(
        backend.retry_unresolved_recovery(&destination).unwrap(),
        Some(Vec::new())
    );

    match fault {
        LiveRenameFault::BeforeMutation => {
            assert!(backend.take_recovery_source_restored(&destination));
            assert!(!backend.take_recovery_source_preserved(&destination));
            assert!(!backend.entry_exists(&destination).unwrap());
        }
        LiveRenameFault::AfterMutation => {
            assert!(backend.take_recovery_source_preserved(&destination));
            assert!(!backend.take_recovery_source_restored(&destination));
            let downloaded = local.path().join("downloaded.bin");
            backend
                .download_file(&destination, &downloaded, None, None)
                .expect("the real SFTP channel must read the recovered destination");
            assert_eq!(fs::read(downloaded).unwrap(), b"live-sftp-payload");
        }
    }
    assert_eq!(fs::read(&local_source).unwrap(), b"live-sftp-payload");
    assert!(backend
        .list_dir(&case_root)
        .unwrap()
        .iter()
        .all(|entry| !entry.name.contains(".zaplex")));

    slot.set(None);
    second_server.abort();
    let _ = second_server.await;
    fs::remove_dir_all(case_root).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the isolated OpenSSH fixture from live-sftp-safety.yml"]
async fn live_sftp_remote_safe_rename_replays_when_request_was_not_applied() {
    verify_live_sftp_rename_recovery(LiveRenameFault::BeforeMutation).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the isolated OpenSSH fixture from live-sftp-safety.yml"]
async fn live_sftp_remote_safe_rename_recovers_when_response_was_lost() {
    verify_live_sftp_rename_recovery(LiveRenameFault::AfterMutation).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the isolated OpenSSH fixture from live-sftp-safety.yml"]
async fn live_sftp_remote_safe_rename_does_not_claim_a_missing_user_source_was_restored() {
    let (host, port, username, key_path, root) = live_sftp_configuration();
    let case_root = root.join(format!("zaplex-live-sftp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&case_root).unwrap();
    let source = case_root.join("source.bin");
    let destination = case_root.join("destination.bin");
    fs::write(&source, b"user-payload").unwrap();

    let session = zap_sftp::SftpSession::connect(
        &host,
        port,
        &username,
        zap_sftp::AuthMethod::PublicKey {
            key_path,
            passphrase: None,
        },
        Some(std::time::Duration::from_secs(10)),
    )
    .expect("the CI OpenSSH fixture must accept the configured key");
    let sftp = session.sftp().expect("the live SFTP subsystem must open");
    let journal = tempdir().unwrap();
    let slot = SafeFileClientSlot::default();
    let (first_client, _first_executor, first_server) = spawn_live_safe_file_client(
        journal.path().to_path_buf(),
        Some(LiveRenameFault::BeforeMutation),
    );
    slot.set(Some(first_client));
    let backend = LiveSftpBackend::new_with_safe_file_slot(sftp, slot.clone());
    let anchor = backend
        .existing_entry_ownership_anchor(&source)
        .unwrap()
        .expect("an existing remote file must have a daemon ownership anchor");

    let error = backend
        .rename_if_matches(&source, &destination, anchor)
        .expect_err("the injected transport loss must require recovery");
    assert!(matches!(error, SftpOpsError::RecoveryRequired { .. }));
    first_server.await.unwrap();
    fs::remove_file(&source).unwrap();

    let (second_client, _second_executor, second_server) =
        spawn_live_safe_file_client(journal.path().to_path_buf(), None);
    slot.set(Some(second_client));
    let error = backend
        .retry_unresolved_recovery(&destination)
        .expect_err("two missing user paths must remain unresolved");
    assert!(error
        .to_string()
        .contains("source and destination are both missing"));
    assert!(!backend.take_recovery_source_restored(&destination));

    slot.set(None);
    second_server.abort();
    let _ = second_server.await;
    fs::remove_dir_all(case_root).unwrap();
}
