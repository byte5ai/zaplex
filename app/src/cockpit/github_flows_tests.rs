//! Tests for the headless GitHub-flow spine: fault-tolerant parsing and
//! shell-safe `gh` command construction. `pick_analysis_instance` is a direct
//! delegation to `zaplex_cockpit::pick_freest` (9 tests in `routing.rs`), so it
//! carries no logic of its own to re-test here.

use super::*;

// ── parse_issue_draft ──────────────────────────────────────────────────────

#[test]
fn issue_draft_parses_fenced_json() {
    let raw = "```json\n{\"title\": \"Fix crash\", \"body\": \"steps...\", \"labels\": [\"bug\"]}\n```";
    let d = parse_issue_draft(raw).expect("fenced draft parses");
    assert_eq!(d.title, "Fix crash");
    assert_eq!(d.body, "steps...");
    assert_eq!(d.labels, vec!["bug"]);
}

#[test]
fn issue_draft_parses_bare_json_and_defaults_labels() {
    let d = parse_issue_draft(r#"{"title": "t", "body": "b"}"#).expect("bare draft parses");
    assert!(d.labels.is_empty(), "labels default to empty");
}

#[test]
fn issue_draft_rejects_empty_title_and_garbage() {
    assert_eq!(parse_issue_draft(r#"{"title": "  ", "body": "b"}"#), None);
    assert_eq!(parse_issue_draft("not json at all"), None);
    assert_eq!(parse_issue_draft(r#"{"body": "no title"}"#), None);
}

// ── parse_triage_verdict ───────────────────────────────────────────────────

#[test]
fn triage_verdict_renames_type_and_defaults() {
    let raw = r#"{"type": "bug", "priority": "high", "actionable": true}"#;
    let v = parse_triage_verdict(raw).expect("triage parses");
    assert_eq!(v.issue_type, "bug");
    assert_eq!(v.priority, "high");
    assert!(v.actionable);
    assert_eq!(v.comment, None); // default
    assert!(!v.close); // default
}

#[test]
fn triage_verdict_full_and_fenced() {
    let raw = "```\n{\"type\":\"question\",\"priority\":\"low\",\"actionable\":false,\
               \"comment\":\"Can you share a repro?\",\"close\":true}\n```";
    let v = parse_triage_verdict(raw).expect("full triage parses");
    assert_eq!(v.comment.as_deref(), Some("Can you share a repro?"));
    assert!(v.close);
}

#[test]
fn triage_verdict_missing_required_field_is_none() {
    // `priority` absent → parse failure (no silent default for a required field).
    assert_eq!(parse_triage_verdict(r#"{"type":"bug","actionable":true}"#), None);
}

// ── parse_pr_review_verdict ────────────────────────────────────────────────

#[test]
fn pr_review_verdict_parses_decision_and_comments() {
    let raw = r#"{"summary":"LGTM with nits","decision":"request_changes",
                 "comments":[{"path":"src/a.rs","line":42,"body":"unwrap here"}]}"#;
    let v = parse_pr_review_verdict(raw).expect("pr review parses");
    assert_eq!(v.decision, PrReviewDecision::RequestChanges);
    assert_eq!(v.comments.len(), 1);
    assert_eq!(v.comments[0].path, "src/a.rs");
    assert_eq!(v.comments[0].line, 42);
}

#[test]
fn pr_review_verdict_defaults_comments_and_maps_variants() {
    let v = parse_pr_review_verdict(r#"{"summary":"ok","decision":"approve"}"#)
        .expect("approve parses");
    assert_eq!(v.decision, PrReviewDecision::Approve);
    assert!(v.comments.is_empty());
    assert_eq!(
        parse_pr_review_verdict(r#"{"summary":"s","decision":"comment"}"#)
            .unwrap()
            .decision,
        PrReviewDecision::Comment
    );
    // Unknown decision variant → parse failure.
    assert_eq!(parse_pr_review_verdict(r#"{"summary":"s","decision":"nuke"}"#), None);
}

// ── gh command builders: shell safety + repo flag ──────────────────────────

#[test]
fn issue_create_quotes_all_values_and_labels() {
    let draft = IssueDraft {
        title: "Crash on `rm -rf`; see $HOME".to_string(),
        body: "line1\nline2 \"quoted\"".to_string(),
        labels: vec!["bug".to_string(), "needs repro".to_string()],
    };
    let cmd = gh_issue_create_cmd(&draft, Some("byte5ai/zaplex"));
    // Repo flag present; hostile title/body/labels are single-quoted, so no
    // metacharacter (`, ;, $, ", space) can escape into the shell.
    assert!(cmd.starts_with("gh issue create --repo byte5ai/zaplex --title "));
    assert!(cmd.contains(r#"--title 'Crash on `rm -rf`; see $HOME'"#));
    assert!(cmd.contains("--label bug"));
    assert!(cmd.contains("--label 'needs repro'"));
    // No unquoted metacharacter leaks: the only `;` is inside the quoted title.
    assert_eq!(cmd.matches("; see").count(), 1);
}

#[test]
fn issue_create_omits_repo_flag_when_none() {
    let draft = IssueDraft {
        title: "t".to_string(),
        body: "b".to_string(),
        labels: vec![],
    };
    let cmd = gh_issue_create_cmd(&draft, None);
    assert_eq!(cmd, "gh issue create --title t --body b");
}

#[test]
fn issue_comment_and_close_commands() {
    assert_eq!(
        gh_issue_comment_cmd(12, "please add a repro", Some("o/r")),
        "gh issue comment 12 --repo o/r --body 'please add a repro'"
    );
    assert_eq!(
        gh_issue_close_cmd(7, Some("duplicate of #3"), None),
        "gh issue close 7 --comment 'duplicate of #3'"
    );
    assert_eq!(gh_issue_close_cmd(7, None, Some("o/r")), "gh issue close 7 --repo o/r");
}

#[test]
fn pr_review_commands_per_decision() {
    assert_eq!(
        gh_pr_review_cmd(5, PrReviewDecision::Approve, None, Some("o/r")),
        "gh pr review 5 --repo o/r --approve"
    );
    assert_eq!(
        gh_pr_review_cmd(5, PrReviewDecision::RequestChanges, Some("fix the unwrap"), None),
        "gh pr review 5 --request-changes --body 'fix the unwrap'"
    );
    assert_eq!(
        gh_pr_review_cmd(9, PrReviewDecision::Comment, Some("nit: naming"), None),
        "gh pr review 9 --comment --body 'nit: naming'"
    );
}

#[test]
fn pr_merge_command_is_squash() {
    assert_eq!(gh_pr_merge_cmd(5, Some("o/r")), "gh pr merge 5 --repo o/r --squash");
    assert_eq!(gh_pr_merge_cmd(5, None), "gh pr merge 5 --squash");
}

// ── flow prompts ───────────────────────────────────────────────────────────

#[test]
fn flow_prompts_keep_human_in_the_loop() {
    // Every flow prompt must require explicit confirmation before running gh —
    // an instance never mutates GitHub on its own.
    for p in [quick_issue_prompt(), pr_review_prompt(), triage_prompt()] {
        let lower = p.to_lowercase();
        assert!(lower.contains("confirm") || lower.contains("go-ahead"),
            "prompt must gate on user confirmation: {p}");
        assert!(p.contains("gh "), "prompt must reference the gh CLI: {p}");
    }
    assert!(quick_issue_prompt().contains("gh issue create"));
    assert!(pr_review_prompt().contains("gh pr review"));
    assert!(triage_prompt().contains("gh issue list"));
}
