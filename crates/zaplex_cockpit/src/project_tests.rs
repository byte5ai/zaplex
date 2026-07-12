//! Tests for git-root project resolution.

use super::*;
use std::fs;
use std::path::Path;

/// Write a minimal `.git/config` with the given origin url under `dir`, plus a
/// `HEAD` on `main` (so branch resolution has something to read).
fn init_repo_with_origin(dir: &Path, url: &str) {
    let git = dir.join(".git");
    fs::create_dir_all(&git).unwrap();
    fs::write(
        git.join("config"),
        format!("[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"),
    )
    .unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
}

#[test]
fn nested_cwd_in_repo_names_from_origin_url() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("checkout");
    init_repo_with_origin(&repo, "git@github.com:iret77/zaplex.git");
    let nested = repo.join("crates").join("zaplex_cockpit").join("src");
    fs::create_dir_all(&nested).unwrap();

    let p = resolve_project(&nested);
    assert_eq!(p.root, repo.to_string_lossy());
    assert_eq!(p.name, "zaplex", "name comes from the origin url basename");
    assert_eq!(p.branch.as_deref(), Some("main"), "branch from .git/HEAD");
    assert_eq!(p.worktree, None, "a primary checkout has no linked-worktree name");
}

#[test]
fn https_origin_url_is_parsed() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("r");
    init_repo_with_origin(
        &repo,
        "https://github.com/byte5ai/engineering-standards.git",
    );
    let p = resolve_project(&repo);
    assert_eq!(p.name, "engineering-standards");
}

#[test]
fn git_without_origin_falls_back_to_dir_basename() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("my-local-repo");
    let git = repo.join(".git");
    fs::create_dir_all(&git).unwrap();
    fs::write(
        git.join("config"),
        "[core]\n\trepositoryformatversion = 0\n",
    )
    .unwrap();

    let p = resolve_project(&repo);
    assert_eq!(p.root, repo.to_string_lossy());
    assert_eq!(p.name, "my-local-repo");
}

#[test]
fn no_git_anywhere_root_is_cwd_name_is_basename() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("just-a-dir");
    fs::create_dir_all(&plain).unwrap();

    let p = resolve_project(&plain);
    assert_eq!(p.root, plain.to_string_lossy());
    assert_eq!(p.name, "just-a-dir");
}

#[test]
fn dot_git_file_worktree_is_treated_as_a_root() {
    // A worktree checkout carries a `.git` *file* (not a dir) pointing at the
    // real gitdir. It must still be detected as a project root.
    let tmp = tempfile::tempdir().unwrap();

    // The main repo + its shared git dir with origin.
    let main = tmp.path().join("main");
    init_repo_with_origin(&main, "git@github.com:iret77/zaplex.git");
    let common_git = main.join(".git");

    // The worktree: a `.git` file + a per-worktree gitdir under the common dir.
    let worktree = tmp.path().join("wt-feature");
    fs::create_dir_all(&worktree).unwrap();
    let wt_gitdir = common_git.join("worktrees").join("wt-feature");
    fs::create_dir_all(&wt_gitdir).unwrap();
    // `commondir` points back to the shared git dir (relative to the gitdir).
    fs::write(wt_gitdir.join("commondir"), "../..\n").unwrap();
    // The worktree's OWN HEAD lives in its per-worktree gitdir (not the shared
    // common dir) — so two worktrees of one repo report distinct branches.
    fs::write(wt_gitdir.join("HEAD"), "ref: refs/heads/feature-x\n").unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", wt_gitdir.to_string_lossy()),
    )
    .unwrap();

    let nested = worktree.join("src");
    fs::create_dir_all(&nested).unwrap();
    let p = resolve_project(&nested);
    assert_eq!(p.root, worktree.to_string_lossy(), "worktree is the root");
    assert_eq!(p.name, "zaplex", "origin resolved via the shared commondir");
    assert_eq!(
        p.branch.as_deref(),
        Some("feature-x"),
        "branch from the per-worktree HEAD, not the common dir"
    );
    assert_eq!(
        p.worktree.as_deref(),
        Some("wt-feature"),
        "linked-worktree name is the gitdir leaf under worktrees/"
    );
}

#[test]
fn detached_head_has_no_branch() {
    // A detached HEAD holds a raw sha, not a `ref:` — branch resolves to None
    // (never a fabricated branch), while the repo still resolves normally.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("detached");
    init_repo_with_origin(&repo, "git@github.com:x/detached.git");
    fs::write(
        repo.join(".git").join("HEAD"),
        "9fceb02d0ae598e95dc970b74767f19372d61af8\n",
    )
    .unwrap();

    let p = resolve_project(&repo);
    assert_eq!(p.name, "detached");
    assert_eq!(p.branch, None, "detached HEAD → no branch");
    assert_eq!(p.worktree, None);
}

#[test]
fn nested_cwd_stops_at_innermost_git_root() {
    // An outer repo containing an inner repo: cwd inside the inner one resolves
    // to the inner root, not the outer.
    let tmp = tempfile::tempdir().unwrap();
    let outer = tmp.path().join("outer");
    init_repo_with_origin(&outer, "git@github.com:x/outer.git");
    let inner = outer.join("vendored").join("inner");
    init_repo_with_origin(&inner, "git@github.com:x/inner.git");

    let p = resolve_project(&inner);
    assert_eq!(p.root, inner.to_string_lossy());
    assert_eq!(p.name, "inner");
}
