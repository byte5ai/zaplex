//! Tests for the pure review-loop spine: the diff-empty decision, command
//! builders (shell-safe), and the read-only Markdown renderer.

use super::*;

// ── WorkingChanges::is_empty ────────────────────────────────────────────────

#[test]
fn empty_when_no_diff_and_no_untracked() {
    assert!(WorkingChanges::default().is_empty());
    assert!(WorkingChanges {
        diff: "\n  \n".to_string(),
        untracked: vec!["".to_string(), "  ".to_string()],
    }
    .is_empty());
}

#[test]
fn not_empty_with_tracked_diff() {
    assert!(!WorkingChanges {
        diff: "diff --git a/x b/x\n".to_string(),
        untracked: vec![],
    }
    .is_empty());
}

#[test]
fn not_empty_with_only_untracked() {
    assert!(!WorkingChanges {
        diff: String::new(),
        untracked: vec!["new.rs".to_string()],
    }
    .is_empty());
}

// ── command builders ────────────────────────────────────────────────────────

#[test]
fn commit_all_cmd_quotes_message_and_path() {
    assert_eq!(
        git_commit_all_cmd("/repo", "fix: bug"),
        "git -C /repo add -A && git -C /repo commit -m 'fix: bug'"
    );
    // No shell quoting needed for a simple path/message.
    assert_eq!(
        git_commit_all_cmd("/tmp/r", "wip"),
        "git -C /tmp/r add -A && git -C /tmp/r commit -m wip"
    );
}

#[test]
fn commit_all_cmd_is_shell_safe() {
    // A hostile commit message must be quoted, not interpreted.
    let cmd = git_commit_all_cmd("/r", "x'; rm -rf / #");
    assert_eq!(
        cmd,
        "git -C /r add -A && git -C /r commit -m 'x'\\''; rm -rf / #'"
    );
    // The message is fully single-quoted after `-m` (never a bare metachar).
    assert!(cmd.contains("commit -m 'x'\\''"));
}

#[test]
fn diff_cmd_targets_head() {
    assert_eq!(git_diff_cmd("/repo"), "git -C /repo diff HEAD");
    assert_eq!(git_diff_cmd("/has space"), "git -C '/has space' diff HEAD");
}

// ── render_review_markdown ──────────────────────────────────────────────────

#[test]
fn renders_calm_no_changes_state() {
    let md = render_review_markdown("zaplex", "main", &WorkingChanges::default(), "");
    assert!(md.contains("# Review — zaplex"));
    assert!(md.contains("Branch: `main`"));
    assert!(md.contains("No working changes"));
    // No diff fence in the empty state.
    assert!(!md.contains("```diff"));
}

#[test]
fn renders_diff_and_untracked_and_commit_preview() {
    let changes = WorkingChanges {
        diff: "diff --git a/x b/x\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        untracked: vec!["new.rs".to_string(), "".to_string()],
    };
    let preview = git_commit_all_cmd("/r", "msg");
    let md = render_review_markdown("proj", "feat/x", &changes, &preview);
    assert!(md.contains("## Untracked files (2)"));
    assert!(md.contains("- `new.rs`"));
    // Blank untracked entry is skipped.
    assert!(!md.contains("- ``"));
    assert!(md.contains("```diff"));
    assert!(md.contains("+new"));
    assert!(md.contains("Commit will run:"));
    assert!(md.contains("git -C /r add -A"));
}

#[test]
fn renders_untracked_only_without_diff_fence() {
    let changes = WorkingChanges {
        diff: String::new(),
        untracked: vec!["a.txt".to_string()],
    };
    let md = render_review_markdown("", "", &changes, "");
    // Falls back to a plain "Review" title, no branch line.
    assert!(md.starts_with("# Review\n"));
    assert!(!md.contains("Branch:"));
    assert!(md.contains("only untracked files"));
    assert!(!md.contains("```diff"));
}
