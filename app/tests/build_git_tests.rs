#[path = "../build_git.rs"]
mod build_git;

#[test]
fn git_watch_paths_exist() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let paths = build_git::watch_paths(manifest_dir);

    assert!(!paths.is_empty());
    assert!(paths.iter().all(|path| path.exists()));
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

#[test]
fn packed_branch_watches_both_packed_refs_and_future_loose_ref() {
    let directory = tempfile::tempdir().unwrap();
    let repo = directory.path();
    let git = |args: &[&str]| {
        let mut command = command::blocking::Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        let output = command
            .current_dir(repo)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--initial-branch=main"]);
    git(&["commit", "--allow-empty", "-m", "first"]);
    git(&["pack-refs", "--all", "--prune"]);
    let loose = repo.join(".git/refs/heads/main");
    assert!(!loose.exists());
    let paths = build_git::watch_paths(repo);
    assert!(paths.contains(&repo.join(".git/packed-refs").canonicalize().unwrap()));
    assert!(paths.contains(&loose.parent().unwrap().canonicalize().unwrap()));
    git(&["commit", "--allow-empty", "-m", "second"]);
    assert!(loose.is_file());
    assert!(build_git::watch_paths(repo).contains(&loose.canonicalize().unwrap()));
}
