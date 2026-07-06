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
    pub root: String,
    /// Human repo label: the `origin` remote's basename when parseable, else
    /// the root directory's basename.
    pub name: String,
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
    let name = origin_repo_name(root_path).unwrap_or_else(|| basename(&root));
    ResolvedProject { root, name }
}

/// First ancestor of `cwd` (inclusive) containing a `.git` entry, if any.
fn git_root(cwd: &Path) -> Option<&Path> {
    let mut cur = Some(cwd);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        cur = dir.parent();
    }
    None
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

/// Parse `<root>/.git/config` for `[remote "origin"] url` and return its repo
/// basename (trailing `.git` stripped, last `/`- or `:`-separated segment).
///
/// Handles a `.git` file (worktree/submodule: `gitdir: <path>`) by following
/// it to the real git dir. Returns `None` if there is no origin url.
fn origin_repo_name(root: &Path) -> Option<String> {
    let git_dir = resolve_git_dir(root)?;
    let config = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let url = origin_url(&config)?;
    Some(repo_name_from_url(&url))
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
