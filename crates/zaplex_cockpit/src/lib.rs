//! zaplex cockpit — the read-only **data spine** for the "plex" half of the product.
//!
//! Discovers Claude Code + Codex accounts/subscriptions, aggregates their own
//! transcript token usage into rolling windows (5h block / today / week), and
//! derives cost (per-model pricing) and heat (load vs. budget).
//!
//! This crate is a **pure, headless-testable data layer**: no GUI, no network, and —
//! a hard privacy invariant — it reads only **token counts and account metadata**,
//! never token strings or transcript content. The `CockpitModel` / file-watch wiring
//! that surfaces this into the app lives in `app/src/cockpit/`.
//!
//! See `docs/superpowers/specs/2026-06-30-cockpit-increment1-account-usage-design.md`.

pub mod claude;
pub mod codex;
pub mod codex_sessions;
pub mod conductor;
pub mod fleet;
pub mod format;
pub mod guardrails;
pub mod oauth;
pub mod overrides;
pub mod pricing;
pub mod project;
pub mod review;
pub mod routing;
pub mod sessions;
pub mod transcript;
pub mod types;
pub mod windows;

pub use conductor::{
    fleet_is_large, fleet_session_count, host_auto_collapsed, host_ident, host_key,
    host_session_count, host_summary, model_effort_label, next_waiting, session_attr_line,
    session_attrs, session_glyph, waiting_sessions, SessionAttrs, WaitingTarget, GLYPH_IDLE,
    GLYPH_WAITING, GLYPH_WORKING,
};
pub use fleet::{
    build_fleet_tree, fold_inventory, AgentSession, FleetTree, HostNode, HostSessions, ProjectNode,
    RemoteHost,
};
pub use format::{
    binding_window, context_fill, context_window, format_cost, format_reset, format_tokens,
    heat_fill, heat_pct_label, heat_pct_label_with_provenance, model_family, HeatLevel,
};
pub use guardrails::{
    failed_toast, guardrail_target, kill_confirm_message, no_remote_connection_toast,
    pid_signalable, remote_unsupported_toast, sent_toast, session_label, stop_all_confirm_message,
    stop_all_summary_toast, unsignalable_toast, GuardrailSignal, GuardrailTarget,
};
pub use oauth::{apply_oauth_usage, parse_oauth_usage, OauthUsage, OauthWindow};
pub use overrides::{AccountOverride, AccountOverrides};
pub use pricing::{ModelPrice, PricingTable};
pub use project::{resolve_project, ResolvedProject};
pub use review::{git_commit_all_cmd, git_diff_cmd, render_review_markdown, WorkingChanges};
pub use routing::{is_over_budget, pick_freest, rank_by_freeness, OVER_BUDGET_HEAT};
pub use transcript::{
    format_transcript_markdown, parse_transcript, ToolCall, TranscriptTurn, TurnRole, TurnUsage,
};
pub use types::{
    Account, AccountStatus, AccountUsage, CockpitSnapshot, Provider, SessionSnapshot, SessionState,
    UsageEntry, UsageProvenance, WindowTotals,
};
pub use windows::{
    build_account_usage, window_5h, window_week, DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK,
};

use std::path::Path;

use chrono::{DateTime, Utc};

/// Build a full cockpit snapshot from disk: discover Claude + Codex accounts, parse
/// their transcripts within the widest (week) window, and aggregate per-account
/// usage / cost / heat.
///
/// `now` is explicit so windowing is deterministic and testable. `budget_5h` /
/// `budget_week` size the two heats (0 = disable). This is the crate's single
/// I/O entry point; the app's `CockpitModel` calls it off the main thread on
/// file-watch/reconcile ticks.
pub fn build_snapshot(
    home: &Path,
    codex_home: &Path,
    claude_config_dir_env: Option<&str>,
    now: DateTime<Utc>,
    budget_5h: u64,
    budget_week: u64,
    pricing: &PricingTable,
) -> CockpitSnapshot {
    let since = now - window_week();
    let mut accounts = Vec::new();

    for account in claude::discover_accounts(home, claude_config_dir_env) {
        let entries = claude::usage_for_account(&account, since);
        let live = sessions::live_sessions(&account.config_dir, now);
        // Explicit user budgets win; otherwise estimate from the plan tier so
        // Enterprise/Team accounts aren't shown falsely maxed.
        let (plan_5h, plan_week) = windows::plan_budgets(account.plan_tier.as_deref());
        let b5h = if budget_5h > 0 { budget_5h } else { plan_5h };
        let bwk = if budget_week > 0 {
            budget_week
        } else {
            plan_week
        };
        let usage = build_account_usage(account, entries, now, b5h, bwk, pricing);
        accounts.push(windows::with_sessions(usage, live));
    }
    for account in codex::discover_accounts(codex_home) {
        let entries = codex::usage_for_account(&account, since);
        let b5h = if budget_5h > 0 {
            budget_5h
        } else {
            DEFAULT_BUDGET_5H
        };
        let bwk = if budget_week > 0 {
            budget_week
        } else {
            DEFAULT_BUDGET_WEEK
        };
        // Codex live agent-sessions (Step 8 parity): transcript-inferred, no
        // registry/pid (see `codex_sessions`). Attached so they flow into the
        // unified Agent-Inventory exactly like Claude's.
        let live = codex_sessions::live_sessions(&account.config_dir, now);
        let usage = build_account_usage(account, entries, now, b5h, bwk, pricing);
        accounts.push(windows::with_sessions(usage, live));
    }

    CockpitSnapshot {
        accounts,
        generated_at: now,
    }
}
