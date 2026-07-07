//! Headless spine for the cockpit **review loop** (Agent-Cockpit step 6).
//!
//! When an agent has changed files, the user reviews the working diff and then
//! approves / redirects / commits / opens a PR — all without leaving zaplex.
//! This module is the pure, unit-testable half of that flow: the *diff-empty
//! decision*, the read-only **diff → Markdown** renderer for the review pane,
//! and the exact **git command builders** (shell-quoted) the commit/PR verbs
//! surface as "here is what will run". No process spawning, no UI, no I/O — the
//! app's workspace handler runs the commands and opens the pane, mirroring how
//! `github_flows` splits command construction from execution.

/// One repository's uncommitted **working changes**: the `git diff` of tracked
/// files plus the list of untracked (new) files. The pair the review pane
/// renders and the diff-empty decision keys off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkingChanges {
    /// `git diff HEAD` (staged + unstaged tracked changes), verbatim.
    pub diff: String,
    /// Untracked, non-ignored file paths (`git status --porcelain` `??` /
    /// `ls-files --others --exclude-standard`).
    pub untracked: Vec<String>,
}

impl WorkingChanges {
    /// True when the repo has **no** working changes: an empty tracked diff and
    /// no untracked files. Whitespace-only diff output (git prints nothing, but
    /// callers may hand us a trailing newline) still counts as empty. Drives the
    /// pane's calm "no changes" state — never open a blank/mutating surface.
    pub fn is_empty(&self) -> bool {
        self.diff.trim().is_empty() && self.untracked.iter().all(|f| f.trim().is_empty())
    }
}

/// Shell-quote one argument for display in a command preview. Kept local (not a
/// dependency on the app's `shell_words`) so this crate stays leaf-level; the
/// rules are the POSIX single-quote escape the previews need.
fn shquote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '@' | '='))
    {
        return s.to_string();
    }
    // Wrap in single quotes; a literal ' becomes '\'' .
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The exact `git -C <root> add -A && git -C <root> commit -m <message>` the
/// **Commit** verb runs, shell-quoted. Surfaced in the review pane so the user
/// sees precisely what will happen before pressing commit (github_flows ethos:
/// show the command). `root` is the repo the review targets.
pub fn git_commit_all_cmd(root: &str, message: &str) -> String {
    format!(
        "git -C {root} add -A && git -C {root} commit -m {msg}",
        root = shquote(root),
        msg = shquote(message),
    )
}

/// The exact `git -C <root> diff HEAD` the review pane's diff is built from.
/// Preview-only companion to [`git_commit_all_cmd`].
pub fn git_diff_cmd(root: &str) -> String {
    format!("git -C {} diff HEAD", shquote(root))
}

/// Render the working changes to a read-only Markdown document for the review
/// pane. Reuses the same "write Markdown, open a code/text pane" mechanism the
/// transcript view uses. An empty change set renders a calm "no changes" state
/// rather than an empty page. `commit_preview` (from [`git_commit_all_cmd`]) is
/// appended as the "what Commit will run" footer when there are changes.
pub fn render_review_markdown(
    project_name: &str,
    branch: &str,
    changes: &WorkingChanges,
    commit_preview: &str,
) -> String {
    let title = if project_name.trim().is_empty() {
        "Review".to_string()
    } else {
        format!("Review — {project_name}")
    };
    let branch_line = if branch.trim().is_empty() {
        String::new()
    } else {
        format!("Branch: `{}`\n\n", branch.trim())
    };

    if changes.is_empty() {
        return format!(
            "# {title}\n\n{branch_line}No working changes — this session's repository is clean.\n"
        );
    }

    let mut out = format!("# {title}\n\n{branch_line}");

    if !changes.untracked.is_empty() {
        out.push_str(&format!(
            "## Untracked files ({})\n\n",
            changes.untracked.len()
        ));
        for f in &changes.untracked {
            if !f.trim().is_empty() {
                out.push_str(&format!("- `{}`\n", f.trim()));
            }
        }
        out.push('\n');
    }

    out.push_str("## Working diff\n\n");
    if changes.diff.trim().is_empty() {
        out.push_str("_(only untracked files — no tracked changes)_\n\n");
    } else {
        out.push_str("```diff\n");
        out.push_str(changes.diff.trim_end());
        out.push_str("\n```\n\n");
    }

    if !commit_preview.trim().is_empty() {
        out.push_str("---\n\nCommit will run:\n\n```sh\n");
        out.push_str(commit_preview.trim());
        out.push_str("\n```\n");
    }

    out
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
