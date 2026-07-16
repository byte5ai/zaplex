//! Tests for git-root project resolution.

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Write a minimal `.git/config` with the given origin url under `dir`, plus a
/// `HEAD` on `main` (so branch resolution has something to read).
/// A `.git` git actually recognises: `HEAD` **and** `refs/` **and** `objects/`.
/// Git wants all three and rejects any two (verified against git), and so does
/// `git_root` — a fixture with fewer would describe a directory that no real
/// checkout looks like, and would only prove the resolver accepts more than git.
fn init_repo_with_origin(dir: &Path, url: &str) {
    let git = dir.join(".git");
    fs::create_dir_all(git.join("refs")).unwrap();
    fs::create_dir_all(git.join("objects")).unwrap();
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

// ── Worktrees belong to their repo (F9) ─────────────────────────────────────

/// Build a real repo with real linked worktrees via `git` itself. The plumbing
/// this resolves (`.git` file → gitdir → `commondir`) is git's own contract, so
/// a hand-made fixture would only prove that we can reproduce our own reading of
/// it. Skips when git is unavailable rather than failing for the wrong reason.
fn real_repo_with_worktrees(tmp: &Path, names: &[&str]) -> Option<PathBuf> {
    let main = tmp.join("zaplex");
    std::fs::create_dir_all(&main).ok()?;
    let git = |args: &[&str], cwd: &Path| -> Option<()> {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .ok()?
            .status
            .success();
        ok.then_some(())
    };
    git(&["init", "-q", "-b", "main"], &main)?;
    git(&["remote", "add", "origin", "https://github.com/byte5ai/zaplex.git"], &main)?;
    std::fs::write(main.join("README"), "x").ok()?;
    git(&["add", "."], &main)?;
    git(&["-c", "user.email=t@x", "-c", "user.name=t", "commit", "-qm", "init"], &main)?;
    for name in names {
        let wt = tmp.join(name);
        git(&["worktree", "add", "-q", "-b", name, wt.to_str()?], &main)?;
    }
    Some(main)
}

/// F9's acceptance, in the words of the spec: three worktrees of zaplex are ONE
/// group "zaplex" — not three projects that happen to share a history.
#[test]
fn worktrees_of_one_repo_resolve_to_one_project() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(main) = real_repo_with_worktrees(tmp.path(), &["wt-a", "wt-b", "wt-c"]) else {
        eprintln!("git unavailable — skipping");
        return;
    };

    let main_p = resolve_project(&main);
    let a = resolve_project(&tmp.path().join("wt-a"));
    let b = resolve_project(&tmp.path().join("wt-b"));
    let c = resolve_project(&tmp.path().join("wt-c"));

    // One grouping key across all four.
    for p in [&a, &b, &c] {
        assert_eq!(
            p.repo_root, main_p.repo_root,
            "a worktree groups under the repo it belongs to"
        );
    }
    assert_eq!(main_p.repo_root, main_p.root, "the main checkout is its own repo root");

    // …named after the repo, from the shared config — not after its own folder.
    for p in [&main_p, &a, &b, &c] {
        assert_eq!(p.name, "zaplex", "every worktree answers with the repo's name");
    }

    // The worktree stays an attribute, and each keeps its OWN tree: a review or a
    // scoped launch must land where the session actually works.
    assert_eq!(a.worktree.as_deref(), Some("wt-a"));
    assert_eq!(a.branch.as_deref(), Some("wt-a"));
    assert_ne!(a.root, main_p.root);
    assert_ne!(a.root, b.root);
    assert_eq!(main_p.worktree, None, "the main checkout is not a linked worktree");
}

/// A sub-directory has always grouped under its repo; the repo key must not
/// change that.
#[test]
fn a_subdirectory_still_groups_under_its_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(main) = real_repo_with_worktrees(tmp.path(), &[]) else {
        return;
    };
    let sub = main.join("crates").join("deep");
    std::fs::create_dir_all(&sub).unwrap();

    let p = resolve_project(&sub);
    assert_eq!(p.root, normalize(&main));
    assert_eq!(p.repo_root, normalize(&main));
    assert_eq!(p.name, "zaplex");
}

/// Outside a repo there is no repo to belong to: both keys are the cwd, so the
/// grouping is unchanged from before.
#[test]
fn a_non_repo_dir_is_its_own_repo_root() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("just-a-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let p = resolve_project(&dir);
    assert_eq!(p.repo_root, p.root);
    assert_eq!(p.worktree, None);
}

/// A directory named `.git` is not a repo, and believing it is has teeth: every
/// session anywhere beneath a stray one gets filed under a project that does not
/// exist, named after whatever directory happens to hold it. This host had
/// exactly that — an empty `/tmp/.git` — which silently made three of this
/// crate's tests fail for a year's worth of the wrong reason.
#[test]
fn an_empty_dot_git_directory_is_not_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let stray = tmp.path().join("not-a-repo");
    fs::create_dir_all(stray.join(".git")).unwrap();
    let work = stray.join("some").join("work");
    fs::create_dir_all(&work).unwrap();

    let p = resolve_project(&work);
    assert_eq!(
        p.root,
        normalize(&work),
        "an empty .git must not swallow the directories under it"
    );
    assert_eq!(p.repo_root, p.root);
    assert_eq!(p.branch, None);
}

/// Git wants HEAD *and* refs *and* objects, and rejects any two of the three
/// (verified against git itself). Anything less is a leftover, not a checkout.
#[test]
fn a_partial_dot_git_is_not_a_repo_either() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, files, dirs) in [
        ("head-only", vec!["HEAD"], vec![]),
        ("no-head", vec![], vec!["refs", "objects"]),
        ("no-objects", vec!["HEAD"], vec!["refs"]),
        ("no-refs", vec!["HEAD"], vec!["objects"]),
    ] {
        let dir = tmp.path().join(name);
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        for f in files {
            fs::write(git.join(f), "ref: refs/heads/main\n").unwrap();
        }
        for d in dirs {
            fs::create_dir_all(git.join(d)).unwrap();
        }
        assert_eq!(
            resolve_project(&dir).root,
            normalize(&dir),
            "{name}: an incomplete .git is not a checkout"
        );
        assert_eq!(resolve_project(&dir).branch, None, "{name}: and has no branch");
    }
}

/// A `.git` file pointing nowhere (a pruned worktree) is a leftover too.
#[test]
fn a_dangling_worktree_pointer_is_not_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pruned");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".git"), "gitdir: /nowhere/that/exists\n").unwrap();

    let p = resolve_project(&dir);
    assert_eq!(p.root, normalize(&dir), "a dangling pointer is not a worktree");
    assert_eq!(p.worktree, None);
}
