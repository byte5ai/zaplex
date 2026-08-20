use super::*;
use crate::cockpit::github_flows::PrReviewComment;
use std::path::PathBuf;

fn repository() -> RepositoryContext {
    RepositoryContext {
        slug: "byte5/zaplex".to_string(),
        worktree: PathBuf::from("/tmp/zaplex"),
        display_label: "zaplex".to_string(),
    }
}

fn target(number: u64) -> GitHubTarget {
    GitHubTarget {
        repository: repository(),
        number,
    }
}

#[test]
fn quick_issue_result_freezes_repository_for_exact_mutation() {
    let result = parse_analysis_result(
        FlowKind::QuickIssue,
        None,
        repository(),
        r#"{"title":"Crash on launch","body":"Reproduction steps","labels":["bug"]}"#,
    )
    .expect("valid issue draft");

    let operation = GitHubFlowDialog::operation_for(&result, MutationKind::CreateIssue)
        .expect("create operation");
    let GitHubOperation::CreateIssue { repository, draft } = operation else {
        panic!("expected create-issue operation");
    };
    assert_eq!(repository.slug, "byte5/zaplex");
    assert_eq!(draft.title, "Crash on launch");
    assert_eq!(draft.labels, ["bug"]);
}

#[test]
fn triage_comment_and_close_are_separate_confirmable_operations() {
    let result = AnalysisResult::IssueTriage {
        target: target(17),
        verdict: TriageVerdict {
            issue_type: "bug".to_string(),
            priority: "high".to_string(),
            actionable: true,
            comment: Some("Thanks, this is reproducible.".to_string()),
            close: true,
        },
    };

    let comment = GitHubFlowDialog::operation_for(&result, MutationKind::CommentIssue)
        .expect("comment operation");
    let close = GitHubFlowDialog::operation_for(&result, MutationKind::CloseIssue)
        .expect("close operation");

    assert!(matches!(comment, GitHubOperation::CommentIssue { .. }));
    assert!(matches!(
        close,
        GitHubOperation::CloseIssue { comment: None, .. }
    ));
    assert_ne!(comment.confirmation_text(), close.confirmation_text());
}

#[test]
fn triage_does_not_offer_an_unproposed_comment() {
    let result = AnalysisResult::IssueTriage {
        target: target(18),
        verdict: TriageVerdict {
            issue_type: "question".to_string(),
            priority: "low".to_string(),
            actionable: false,
            comment: None,
            close: false,
        },
    };

    assert_eq!(
        GitHubFlowDialog::operation_for(&result, MutationKind::CommentIssue),
        None
    );
}

#[test]
fn pull_request_review_body_preserves_inline_findings() {
    let result = AnalysisResult::PullRequestReview {
        target: target(21),
        verdict: PrReviewVerdict {
            summary: "One correctness issue.".to_string(),
            decision: PrReviewDecision::RequestChanges,
            comments: vec![PrReviewComment {
                path: "app/src/main.rs".to_string(),
                line: 42,
                body: "This can panic.".to_string(),
            }],
        },
    };

    let operation = GitHubFlowDialog::operation_for(&result, MutationKind::SubmitReview)
        .expect("review operation");
    let GitHubOperation::ReviewPullRequest { body, decision, .. } = operation else {
        panic!("expected review operation");
    };
    assert_eq!(decision, PrReviewDecision::RequestChanges);
    let body = body.expect("review body");
    assert!(body.contains("One correctness issue."));
    assert!(body.contains("app/src/main.rs:42"));
    assert!(body.contains("This can panic."));
}

#[test]
fn mutation_kind_is_exhaustive_for_typed_operations() {
    let operations = [
        (
            GitHubOperation::CreateIssue {
                repository: repository(),
                draft: IssueDraft {
                    title: "Title".to_string(),
                    body: "Body".to_string(),
                    labels: Vec::new(),
                },
            },
            MutationKind::CreateIssue,
        ),
        (
            GitHubOperation::CommentIssue {
                target: target(1),
                body: "Comment".to_string(),
            },
            MutationKind::CommentIssue,
        ),
        (
            GitHubOperation::CloseIssue {
                target: target(2),
                comment: None,
            },
            MutationKind::CloseIssue,
        ),
        (
            GitHubOperation::ReviewPullRequest {
                target: target(3),
                decision: PrReviewDecision::Approve,
                body: Some("Looks good.".to_string()),
            },
            MutationKind::SubmitReview,
        ),
        (
            GitHubOperation::MergePullRequest { target: target(4) },
            MutationKind::MergePullRequest,
        ),
    ];

    for (operation, expected) in operations {
        assert_eq!(mutation_kind(&operation), expected);
    }
}

#[test]
fn parsing_rejects_target_mismatch() {
    let error = parse_analysis_result(
        FlowKind::IssueTriage,
        None,
        repository(),
        r#"{"type":"bug","priority":"high","actionable":true,"close":false}"#,
    )
    .expect_err("missing target must fail");

    assert!(matches!(error, GitHubFlowError::InvalidOutput(_)));
}

#[test]
fn empty_target_lists_are_distinct_from_failures() {
    assert!(matches!(
        target_list_state(Ok(TargetList::Issues(Vec::new()))),
        DialogState::Empty(message) if message.contains("No open issues")
    ));
    assert!(matches!(
        target_list_state(Err(GitHubFlowError::CommandFailed(
            "authentication failed".to_string()
        ))),
        DialogState::Error(message) if message == "authentication failed"
    ));
}
