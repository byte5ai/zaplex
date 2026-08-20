use super::*;
use chrono::TimeZone as _;

fn at(second: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(second, 0).single().unwrap()
}

#[test]
fn mru_is_deduplicated_and_bounded_per_host() {
    let mut history = FolderHistory::empty();
    for index in 0..MAX_FOLDERS_PER_HOST + 5 {
        history
            .record_success(
                &FolderHistoryHost::Local,
                Path::new(&format!("/work/{index}")),
                at(index as i64),
            )
            .unwrap();
    }
    history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/7"), at(99))
        .unwrap();

    let entries = history.entries(&FolderHistoryHost::Local);
    assert_eq!(entries.len(), MAX_FOLDERS_PER_HOST);
    assert_eq!(entries[0].path, PathBuf::from("/work/7"));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.path == Path::new("/work/7"))
            .count(),
        1
    );
}

#[test]
fn local_and_remote_hosts_never_share_history() {
    let mut history = FolderHistory::empty();
    let remote_a = FolderHistoryHost::remote("node-a").unwrap();
    let remote_b = FolderHistoryHost::remote("node-b").unwrap();
    history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/local"), at(1))
        .unwrap();
    history
        .record_success(&remote_a, Path::new("/srv/a"), at(2))
        .unwrap();
    history
        .record_success(&remote_b, Path::new("/srv/b"), at(3))
        .unwrap();

    assert_eq!(
        history.entries(&FolderHistoryHost::Local)[0].path,
        Path::new("/work/local")
    );
    assert_eq!(history.entries(&remote_a)[0].path, Path::new("/srv/a"));
    assert_eq!(history.entries(&remote_b)[0].path, Path::new("/srv/b"));
}

#[test]
fn navigation_truncates_forward_branch_without_reordering_mru() {
    let mut navigation = FolderNavigation::default();
    navigation.reset(Some(PathBuf::from("/a")));
    navigation.select(PathBuf::from("/b"));
    navigation.select(PathBuf::from("/c"));
    assert_eq!(navigation.back(), Some(Path::new("/b")));
    navigation.select(PathBuf::from("/d"));
    assert!(!navigation.can_forward());
    assert_eq!(navigation.back(), Some(Path::new("/b")));
    assert_eq!(navigation.back(), Some(Path::new("/a")));
}

#[test]
fn search_is_case_insensitive_and_does_not_change_selection() {
    let mut history = FolderHistory::empty();
    history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/Zaplex"), at(1))
        .unwrap();
    history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/Other"), at(2))
        .unwrap();

    let matches = history.search(&FolderHistoryHost::Local, "zap");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, Path::new("/work/Zaplex"));
}

#[test]
fn late_validation_result_cannot_enable_a_new_path() {
    let mut state = DirectoryValidationState::default();
    let old = state.begin(FolderHistoryHost::Local, PathBuf::from("/old"));
    let current = state.begin(FolderHistoryHost::Local, PathBuf::from("/current"));
    assert!(!state.apply(&old, DirectoryValidation::Valid));
    assert!(!state.is_valid());
    assert!(state.apply(&current, DirectoryValidation::Valid));
    assert!(state.is_valid());
}

#[test]
fn corrupt_history_is_protected_from_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.json");
    std::fs::write(&path, "not json").unwrap();
    let (history, state) = load_history_from(&path);
    assert_eq!(state, HistoryFileState::Protected);
    assert!(save_history_to(&path, &history, state).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "not json");
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn failed_history_save_does_not_publish_an_unpersisted_entry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.json");
    std::fs::write(&path, "not json").unwrap();
    let mut history = FolderHistory::with_file(path);

    assert!(history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/zaplex"), at(7),)
        .is_err());
    assert!(history.entries(&FolderHistoryHost::Local).is_empty());
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn successful_history_survives_store_reload() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("history.json");
    let mut history = FolderHistory::with_file(file.clone());
    history
        .record_success(&FolderHistoryHost::Local, Path::new("/work/zaplex"), at(7))
        .unwrap();

    let reloaded = FolderHistory::with_file(file);
    assert_eq!(
        reloaded.entries(&FolderHistoryHost::Local)[0].path,
        Path::new("/work/zaplex")
    );
}

#[test]
fn path_normalization_rejects_relative_and_root_escape_paths() {
    assert!(normalize_path(&FolderHistoryHost::Local, Path::new("relative")).is_err());
    assert!(normalize_path(&FolderHistoryHost::Local, Path::new("/../../escape")).is_err());
    assert_eq!(
        normalize_path(
            &FolderHistoryHost::Local,
            Path::new("/work/./zaplex/../app")
        )
        .unwrap(),
        PathBuf::from("/work/app")
    );
}

#[test]
fn remote_paths_use_posix_normalization_independent_of_local_host_syntax() {
    let remote = FolderHistoryHost::remote("node-a").unwrap();
    assert_eq!(
        normalize_path(&remote, Path::new("/srv/./apps/../zaplex")).unwrap(),
        PathBuf::from("/srv/zaplex")
    );
    assert!(normalize_path(&remote, Path::new("srv/zaplex")).is_err());
    assert!(normalize_path(&remote, Path::new("/../../escape")).is_err());
}
