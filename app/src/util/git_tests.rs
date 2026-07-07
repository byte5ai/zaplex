use std::path::Path;

use command::r#async::Command;
use command::Stdio;
use tempfile::TempDir;

use super::{detect_current_branch, detect_current_branch_display, get_review_working_changes};

/// Helper: run a git command inside the given repo directory.
async fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .expect("failed to run git");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Creates a temp git repo with one commit and returns `(dir_handle, repo_path)`.
async fn init_repo() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().to_path_buf();

    git(&path, &["init", "-b", "main"]).await;
    git(&path, &["config", "user.email", "test@test.com"]).await;
    git(&path, &["config", "user.name", "Test"]).await;
    git(&path, &["commit", "--allow-empty", "-m", "initial"]).await;

    (dir, path)
}

#[tokio::test]
async fn on_normal_branch_returns_branch_name() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "-b", "feature-xyz"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "feature-xyz");
    assert_eq!(
        detect_current_branch_display(&repo).await.unwrap(),
        "feature-xyz"
    );
}

#[tokio::test]
async fn detached_head_raw_returns_head() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    assert_eq!(detect_current_branch(&repo).await.unwrap(), "HEAD");
}

#[tokio::test]
async fn detached_head_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["checkout", "--detach", "HEAD"]).await;

    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(
        result, "HEAD",
        "display variant should not return literal HEAD"
    );
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

#[tokio::test]
async fn detached_tag_display_returns_short_sha() {
    let (_dir, repo) = init_repo().await;
    git(&repo, &["tag", "v1.0"]).await;
    git(&repo, &["checkout", "v1.0"]).await;

    let full_sha = git(&repo, &["rev-parse", "HEAD"]).await;
    let result = detect_current_branch_display(&repo).await.unwrap();

    assert_ne!(result, "HEAD");
    assert!(
        full_sha.starts_with(&result),
        "expected {full_sha} to start with {result}"
    );
}

#[tokio::test]
async fn review_changes_before_first_commit_includes_staged_file() {
    // A brand-new repo with no commit yet (unlike `init_repo`, which commits
    // immediately). Files `git add`ed here are staged for the *initial*
    // commit, so there is no HEAD to diff against.
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let repo = dir.path().to_path_buf();
    git(&repo, &["init", "-b", "main"]).await;
    git(&repo, &["config", "user.email", "test@test.com"]).await;
    git(&repo, &["config", "user.name", "Test"]).await;

    // Staged, tracked-but-never-committed file.
    tokio::fs::write(repo.join("staged.txt"), "staged content\n")
        .await
        .expect("failed to write staged file");
    git(&repo, &["add", "staged.txt"]).await;

    // Unstaged edit to a different file that was also staged (never
    // committed), to make sure staged + unstaged don't clobber each other.
    tokio::fs::write(repo.join("mixed.txt"), "v1\n")
        .await
        .expect("failed to write mixed file");
    git(&repo, &["add", "mixed.txt"]).await;
    tokio::fs::write(repo.join("mixed.txt"), "v1\nv2\n")
        .await
        .expect("failed to edit mixed file");

    // Untracked file, never `git add`ed.
    tokio::fs::write(repo.join("untracked.txt"), "untracked content\n")
        .await
        .expect("failed to write untracked file");

    let (diff, untracked) = get_review_working_changes(&repo).await;

    assert!(
        diff.contains("staged.txt") && diff.contains("staged content"),
        "expected staged file to appear in the no-HEAD review diff, got:\n{diff}"
    );
    assert!(
        diff.contains("mixed.txt") && diff.contains("+v2"),
        "expected the unstaged edit on top of the staged file to appear, got:\n{diff}"
    );
    assert!(
        !diff.contains("untracked.txt"),
        "untracked files must come from the untracked list, not the diff, got:\n{diff}"
    );
    assert_eq!(untracked, vec!["untracked.txt".to_string()]);
}
