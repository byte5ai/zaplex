//! Git-root project resolution for the Agent-Inventory.
//!
//! An agent-session's raw `cwd` is often a nested sub-directory of a repo; the
//! Agent-Inventory groups sessions by their **project** — the enclosing git
//! working tree. This module walks up from a `cwd` to the git root and derives
//! a human repo label (preferring the `origin` remote's name).
//!
//! Filesystem reads only (no network, no git subprocess): a `.git` probe up the
//! ancestry plus a best-effort parse of `.git/config`.

use std::path::Path;

/// The resolved project of a working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedProject {
    /// Git working-tree root of the `cwd` (normalized, no trailing slash), or
    /// the `cwd` itself when it is not inside a repo.
    ///
    /// **This session's own tree** — for a linked worktree, the worktree, not the
    /// repo it belongs to. Anything that acts on the files the session sees
    /// (review, a launch scoped to where it works) must use this.
    pub root: String,
    /// Working-tree root of the **repo** `root` belongs to: the main checkout for
    /// a linked worktree, `root` itself otherwise.
    ///
    /// The grouping key for the inventory. Three worktrees of one repo are three
    /// `root`s but one `repo_root`, so they read as one project with three
    /// sessions rather than three unrelated projects that happen to share a name.
    pub repo_root: String,
    /// Human repo label: the `origin` remote's basename when parseable, else
    /// the root directory's basename.
    pub name: String,
    /// Current git branch at `root` (`.git/HEAD` → `refs/heads/<branch>`), or
    /// `None` when detached / unknown / not a repo. The session's primary
    /// identity signal in the redesigned sidebar (§2.2).
    pub branch: Option<String>,
    /// Linked-worktree name when `root` is a git worktree distinct from the
    /// main checkout (`.git` *file* → `worktrees/<name>`), else `None` (the
    /// primary worktree or a non-repo dir).
    pub worktree: Option<String>,
}

/// Resolve `cwd` to its enclosing project (git root + repo label).
///
/// Walks up from `cwd` for the first ancestor holding a `.git` entry — a
/// directory *or* a file (git worktrees use a `.git` file). If none is found,
/// the root is `cwd` itself. The `name` prefers the repo name parsed from the
/// root's `.git/config` `[remote "origin"] url`; on failure it falls back to
/// the root directory's basename.
pub fn resolve_project(cwd: &Path) -> ResolvedProject {
    let root_path = git_root(cwd).unwrap_or(cwd);
    let root = normalize(root_path);
    // The repo this tree belongs to. A linked worktree points at the main
    // checkout; everything else is its own repo.
    let repo_root = main_worktree_root(root_path)
        .map(|p| normalize(&p))
        .unwrap_or_else(|| root.clone());
    // Named after the REPO, not this tree: `origin` is recorded once, in the
    // shared config, so every worktree of a repo answers with the same name.
    let name = origin_repo_name(root_path).unwrap_or_else(|| basename(&repo_root));
    let (branch, worktree) = resolve_worktree_identity(root_path);
    ResolvedProject {
        root,
        repo_root,
        name,
        branch,
        worktree,
    }
}

/// The main checkout of the repo `root` belongs to, or `None` when `root` is
/// already it (a primary checkout) or is not a repo at all.
///
/// Git states the relationship itself: a linked worktree's per-worktree gitdir
/// holds a `commondir` file pointing at the repo's shared git dir, whose parent
/// is the main working tree. Read that rather than stripping `worktrees/<name>`
/// off the gitdir by hand — the two agree for an ordinary layout, but only the
/// file is the contract, and it keeps holding when the git dir lives somewhere
/// else entirely (`--separate-git-dir`).
fn main_worktree_root(root: &Path) -> Option<std::path::PathBuf> {
    // Not a checkout → no repo above it. Guarding here too keeps a dangling
    // `.git` file from producing a repo_root under a path that does not exist:
    // a grouping key made of leftovers, quietly collecting unrelated sessions.
    if !is_work_tree(root) {
        return None;
    }
    let dot_git = root.join(".git");
    // A directory means this IS the main checkout; nothing to resolve.
    if std::fs::metadata(&dot_git).ok()?.is_dir() {
        return None;
    }
    let gitdir = worktree_gitdir(root)?;
    let common = git_common_dir(&gitdir);
    // `<main>/.git` → `<main>`. A bare repo has no working tree above it, so its
    // parent is not one; grouping still keys on a path every worktree of the repo
    // shares, which is all the key has to do.
    let common = common.canonicalize().unwrap_or(common);
    common.parent().map(Path::to_path_buf)
}

/// The per-worktree git dir a `.git` *file* points at (`gitdir: <path>`).
fn worktree_gitdir(root: &Path) -> Option<std::path::PathBuf> {
    let contents = std::fs::read_to_string(root.join(".git")).ok()?;
    let gitdir = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)?;
    let p = Path::new(gitdir);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    })
}

/// Resolve `(branch, worktree)` for a working-tree root by reading Git plumbing
/// files directly (no subprocess, matching this module's filesystem-only
/// contract): the branch from the (per-worktree) `HEAD`, and the linked-worktree
/// name from a `.git` *file*'s `gitdir: …/worktrees/<name>`.
///
/// A primary checkout (`.git` is a directory) has a branch but no linked-worktree
/// name; a linked worktree (`.git` is a file) has both, read from its own gitdir
/// under `worktrees/<name>` — **not** the shared common dir, so two worktrees of
/// one repo report their own distinct branch.
fn resolve_worktree_identity(root: &Path) -> (Option<String>, Option<String>) {
    // Only a real checkout has an identity to report. Without this, a stray
    // `.git` holding nothing but a HEAD would hand back a branch for a directory
    // that is not a repo, and a `.git` file pointing at a pruned worktree would
    // name one after the dead path's last segment — facts invented out of
    // leftovers. `git_root` refuses to call either a work tree; this must agree,
    // or `root` and the identity beside it describe different things.
    if !is_work_tree(root) {
        return (None, None);
    }
    let dot_git = root.join(".git");
    let Ok(meta) = std::fs::metadata(&dot_git) else {
        return (None, None);
    };
    if meta.is_dir() {
        // Primary worktree: HEAD lives directly under `.git`; no linked name.
        return (head_branch(&dot_git), None);
    }
    // Linked worktree: `.git` is a file `gitdir: <path>/worktrees/<name>`.
    let Ok(contents) = std::fs::read_to_string(&dot_git) else {
        return (None, None);
    };
    let Some(gitdir) = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)
    else {
        return (None, None);
    };
    let gitdir_path = Path::new(gitdir);
    let gitdir_abs = if gitdir_path.is_absolute() {
        gitdir_path.to_path_buf()
    } else {
        root.join(gitdir_path)
    };
    let branch = head_branch(&gitdir_abs);
    // The worktree's own name is the leaf of its per-worktree gitdir.
    let worktree = gitdir_abs
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string);
    (branch, worktree)
}

/// The current branch from a git dir's `HEAD` (`ref: refs/heads/<branch>`), or
/// `None` when detached (HEAD holds a raw sha) or the file is unreadable.
fn head_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

/// First ancestor of `cwd` (inclusive) that is a git working tree, if any.
fn git_root(cwd: &Path) -> Option<&Path> {
    let mut cur = Some(cwd);
    while let Some(dir) = cur {
        if is_work_tree(dir) {
            return Some(dir);
        }
        cur = dir.parent();
    }
    None
}

/// Is `dir` a git working tree — i.e. would git itself say so?
///
/// The presence of something named `.git` is **not** enough, though it is the
/// obvious test and this used to make it. An empty directory called `.git` — an
/// aborted `git init`, a half-finished copy, some other tool's leftovers — made
/// every directory beneath it look like one repo. Sessions from unrelated work
/// were then filed under a project that does not exist, named after whatever
/// directory happened to hold the stray. (This host has exactly that at
/// `/tmp/.git`, which is what kept three of this crate's tests red.)
///
/// So apply git's own rule instead, verified against git:
/// - a `.git` **directory** must hold `HEAD`, `refs/` and `objects/` — git wants
///   all three, and rejects any two of them;
/// - a `.git` **file** must say `gitdir: …` and point at a directory holding a
///   `HEAD` (a linked worktree or submodule; its `refs`/`objects` live in the
///   repo's shared common dir, so they are not here to check).
fn is_work_tree(dir: &Path) -> bool {
    let dot_git = dir.join(".git");
    let Ok(meta) = std::fs::metadata(&dot_git) else {
        return false;
    };
    if meta.is_dir() {
        return dot_git.join("HEAD").exists()
            && dot_git.join("refs").exists()
            && dot_git.join("objects").exists();
    }
    worktree_gitdir(dir).is_some_and(|gitdir| gitdir.join("HEAD").exists())
}

/// Normalize a path to a string without a trailing slash (root `/` preserved).
fn normalize(p: &Path) -> String {
    let s = p.to_string_lossy();
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Final path component of a normalized path string.
fn basename(root: &str) -> String {
    root.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(root)
        .to_string()
}

/// Parse the repo's git config for `[remote "origin"] url` and return its repo
/// basename (trailing `.git` stripped, last `/`- or `:`-separated segment).
///
/// Reads the **shared** git dir, not the per-worktree one. `config` is recorded
/// once per repo and lives in the common dir; a linked worktree's own gitdir has
/// no `config` at all, so looking there found nothing and every worktree fell
/// back to being named after its own directory — one repo showing up as
/// "zaplex" and "rc-master-plan", two projects that share nothing but a name.
/// Returns `None` if there is no origin url.
fn origin_repo_name(root: &Path) -> Option<String> {
    let git_dir = resolve_git_dir(root)?;
    let config = std::fs::read_to_string(git_common_dir(&git_dir).join("config")).ok()?;
    let url = origin_url(&config)?;
    Some(repo_name_from_url(&url))
}

/// The repo's shared git dir for a (possibly per-worktree) git dir: follows the
/// `commondir` file when there is one, else `git_dir` itself is already shared.
fn git_common_dir(git_dir: &Path) -> std::path::PathBuf {
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => git_dir.join(rel.trim()),
        Err(_) => git_dir.to_path_buf(),
    }
}

/// Resolve the real git directory for a working-tree root: `<root>/.git` when a
/// directory, or the `gitdir:` target when `<root>/.git` is a file.
fn resolve_git_dir(root: &Path) -> Option<std::path::PathBuf> {
    let dot_git = root.join(".git");
    let meta = std::fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    // `.git` file: `gitdir: <path>` (absolute, or relative to the worktree).
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let target = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)?;
    let target_path = Path::new(target);
    let resolved = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        root.join(target_path)
    };
    // A worktree's gitdir is `<common>/worktrees/<name>`; its own `config` is
    // shared via the `commondir` file. Follow it so origin is discoverable.
    let common = resolved.join("commondir");
    if let Ok(rel) = std::fs::read_to_string(&common) {
        let rel = rel.trim();
        let common_dir = resolved.join(rel);
        if let Ok(c) = common_dir.canonicalize() {
            return Some(c);
        }
    }
    Some(resolved)
}

/// Extract the `[remote "origin"]` `url = ...` value from a git config body.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for raw in config.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_origin = section_is_origin(line);
            continue;
        }
        if in_origin {
            if let Some(rest) = line.strip_prefix("url") {
                let rest = rest.trim_start();
                if let Some(val) = rest.strip_prefix('=') {
                    return Some(val.trim().to_string());
                }
            }
        }
    }
    None
}

/// True for `[remote "origin"]` in either the classic or subsection form.
fn section_is_origin(header: &str) -> bool {
    let inner = header.trim_start_matches('[').trim_end_matches(']').trim();
    inner == "remote \"origin\"" || inner == "remote 'origin'"
}

/// Repo name from a remote url: strip a trailing `.git`, take the last segment
/// split on `/` or `:` (covers `git@host:owner/repo.git` and https forms).
fn repo_name_from_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let stripped = url.strip_suffix(".git").unwrap_or(url);
    stripped
        .rsplit(['/', ':'])
        .find(|s| !s.is_empty())
        .unwrap_or(stripped)
        .to_string()
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
