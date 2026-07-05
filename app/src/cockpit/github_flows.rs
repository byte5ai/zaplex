//! Headless core for the GitHub instance-driven flows (C5, audit (e)).
//!
//! The three flows — **Quick-Issue** (draft → review → `gh issue create`),
//! **PR-Review** (list → analyze → approve/comment/merge), and **Issue-Triage**
//! (type/priority/actionable → comment/close) — all share the same spine:
//!
//! 1. pick the **freest** agent instance to run the background analysis on, so
//!    it never steals capacity from a foreground session ([`pick_analysis_instance`],
//!    reusing the C4 routing engine);
//! 2. parse the instance's **fault-tolerant fenced JSON** into a typed verdict
//!    (reusing the active-AI [`strip_code_fence`] parser — models fence and
//!    over-explain, so strict parsing would drop good output); and
//! 3. turn the verdict into an exact, **shell-safe `gh` command** — every
//!    interpolated value quoted, so hostile titles/bodies can't break out.
//!
//! This module is the pure data/command spine (no process spawning, no UI): it
//! is fully unit-testable, mirroring how `zaplex_cockpit::routing` (C4-1) landed
//! before its UI. Execution + surfaces build on top of it.

use crate::ai::agent_providers::active_ai::parsing::strip_code_fence;
use serde::Deserialize;
use zaplex_cockpit::{AccountUsage, Provider};

/// A drafted issue awaiting the user's review before `gh issue create`
/// (Quick-Issue flow). The instance proposes; the user disposes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueDraft {
    pub title: String,
    pub body: String,
    /// Suggested labels (may be empty; only applied if they exist on the repo).
    #[serde(default)]
    pub labels: Vec<String>,
}

/// The instance's triage verdict for an existing issue (Issue-Triage flow).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TriageVerdict {
    /// Classified type, e.g. "bug" / "feature" / "question" / "docs".
    #[serde(rename = "type")]
    pub issue_type: String,
    /// Classified priority, e.g. "low" / "medium" / "high".
    pub priority: String,
    /// Whether the issue is actionable as written (enough detail to start).
    pub actionable: bool,
    /// Optional triage comment to post (e.g. asking for a repro).
    #[serde(default)]
    pub comment: Option<String>,
    /// Whether the instance recommends closing (e.g. duplicate / not-a-bug).
    #[serde(default)]
    pub close: bool,
}

/// What the instance recommends doing with a pull request (PR-Review flow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrReviewDecision {
    Approve,
    Comment,
    RequestChanges,
}

/// One inline review comment anchored to a file + line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PrReviewComment {
    pub path: String,
    pub line: u32,
    pub body: String,
}

/// The instance's PR-review verdict.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PrReviewVerdict {
    pub summary: String,
    pub decision: PrReviewDecision,
    #[serde(default)]
    pub comments: Vec<PrReviewComment>,
}

/// Pick the agent instance a **background GitHub analysis** should run on: the
/// *freest* one for the provider, so a long triage/review never competes with
/// whatever the user is actively doing. Thin semantic wrapper over the C4
/// routing engine (`pick_freest` already deprioritizes working/over-budget
/// accounts) — kept as its own name so call sites read as intent, and so the
/// selection policy for analyses can diverge later without touching launch.
pub fn pick_analysis_instance<'a>(
    provider: Provider,
    accounts: &'a [AccountUsage],
) -> Option<&'a AccountUsage> {
    zaplex_cockpit::pick_freest(provider, accounts)
}

/// Parse an instance's Quick-Issue output into an [`IssueDraft`]. Fault-tolerant:
/// strips code fences first, returns `None` on any parse failure (caller surfaces
/// a retry rather than creating a malformed issue).
pub fn parse_issue_draft(raw: &str) -> Option<IssueDraft> {
    let draft: IssueDraft = serde_json::from_str(strip_code_fence(raw)).ok()?;
    // A titleless issue is never valid — treat as a parse failure.
    if draft.title.trim().is_empty() {
        return None;
    }
    Some(draft)
}

/// Parse an instance's Issue-Triage output into a [`TriageVerdict`]. See
/// [`parse_issue_draft`] for the fault-tolerance contract.
pub fn parse_triage_verdict(raw: &str) -> Option<TriageVerdict> {
    serde_json::from_str(strip_code_fence(raw)).ok()
}

/// Parse an instance's PR-Review output into a [`PrReviewVerdict`]. See
/// [`parse_issue_draft`] for the fault-tolerance contract.
pub fn parse_pr_review_verdict(raw: &str) -> Option<PrReviewVerdict> {
    serde_json::from_str(strip_code_fence(raw)).ok()
}

/// `--repo <owner/name>` fragment, or empty when `repo` is `None` (let `gh` use
/// the cwd's repository). Repo slugs are `owner/name`, but quote defensively.
fn repo_flag(repo: Option<&str>) -> String {
    match repo {
        Some(r) => format!(" --repo {}", shell_words::quote(r)),
        None => String::new(),
    }
}

/// Build the exact `gh issue create` command for a reviewed draft. Every value
/// is shell-quoted, so a title/body/label containing spaces, quotes, or shell
/// metacharacters is passed literally — never interpreted.
pub fn gh_issue_create_cmd(draft: &IssueDraft, repo: Option<&str>) -> String {
    let mut cmd = format!(
        "gh issue create{} --title {} --body {}",
        repo_flag(repo),
        shell_words::quote(&draft.title),
        shell_words::quote(&draft.body),
    );
    for label in &draft.labels {
        cmd.push_str(&format!(" --label {}", shell_words::quote(label)));
    }
    cmd
}

/// Build `gh issue comment <number> --body <comment>`.
pub fn gh_issue_comment_cmd(number: u64, comment: &str, repo: Option<&str>) -> String {
    format!(
        "gh issue comment {}{} --body {}",
        number,
        repo_flag(repo),
        shell_words::quote(comment),
    )
}

/// Build `gh issue close <number>` (with an optional closing comment).
pub fn gh_issue_close_cmd(number: u64, comment: Option<&str>, repo: Option<&str>) -> String {
    let mut cmd = format!("gh issue close {}{}", number, repo_flag(repo));
    if let Some(c) = comment {
        cmd.push_str(&format!(" --comment {}", shell_words::quote(c)));
    }
    cmd
}

/// Build `gh pr review <number>` for a decision, with an optional body.
/// `gh` requires a body for `--comment` / `--request-changes`; the caller is
/// responsible for supplying one there (the flow's verdict always has a summary).
pub fn gh_pr_review_cmd(
    number: u64,
    decision: PrReviewDecision,
    body: Option<&str>,
    repo: Option<&str>,
) -> String {
    let verb = match decision {
        PrReviewDecision::Approve => "--approve",
        PrReviewDecision::Comment => "--comment",
        PrReviewDecision::RequestChanges => "--request-changes",
    };
    let mut cmd = format!("gh pr review {}{} {}", number, repo_flag(repo), verb);
    if let Some(b) = body {
        cmd.push_str(&format!(" --body {}", shell_words::quote(b)));
    }
    cmd
}

/// Build `gh pr merge <number> --squash`. Squash keeps the merged history linear
/// (matches the repo's PR workflow); the caller gates this behind explicit user
/// confirmation — a merge is never auto-issued from an analysis.
pub fn gh_pr_merge_cmd(number: u64, repo: Option<&str>) -> String {
    format!("gh pr merge {}{} --squash", number, repo_flag(repo))
}

#[cfg(test)]
#[path = "github_flows_tests.rs"]
mod tests;
