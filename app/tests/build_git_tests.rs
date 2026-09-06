#[path = "../build_git.rs"]
mod build_git;

#[test]
fn git_watch_paths_exist() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let paths = build_git::watch_paths(manifest_dir);

    assert!(!paths.is_empty());
    assert!(paths.iter().all(|path| path.is_file()));
}

#[test]
fn existing_path_rejects_missing_git_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let manifest_dir = directory.path().join("app");
    let git_dir = directory.path().join(".git");
    std::fs::create_dir_all(&manifest_dir).unwrap();
    std::fs::create_dir_all(&git_dir).unwrap();
    let head = git_dir.join("HEAD");
    std::fs::write(&head, "ref: refs/heads/main\n").unwrap();

    assert_eq!(
        build_git::existing_path(&manifest_dir, "../.git/HEAD"),
        Some(head.canonicalize().unwrap())
    );
    assert_eq!(build_git::existing_path(&manifest_dir, ".git/HEAD"), None);
}
