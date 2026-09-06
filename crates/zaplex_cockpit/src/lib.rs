//! zaplex cockpit — the read-only **data spine** for the "plex" half of the product.
//!
//! Discovers Claude Code + Codex accounts/subscriptions, aggregates their own
//! transcript token usage into rolling windows (5h block / today / week), and
//! derives cost (per-model pricing) and heat (load vs. budget).
//!
//! This crate is a **pure, headless-testable data layer**: no GUI and no network.
//! It never reads token strings or credentials. Structured task titles are the
//! one deliberate transcript-content projection: providers emit them through
//! task tools, and session snapshots carry them for the Conductor. The
//! `CockpitModel` / file-watch wiring that surfaces this into the app lives in
//! `app/src/cockpit/`.
//!
//! See `docs/superpowers/specs/2026-06-30-cockpit-increment1-account-usage-design.md`.

pub mod antigravity_sessions;
pub mod claude;
pub mod claude_registry_lifecycle;
pub mod codex;
pub mod codex_sessions;
pub mod conductor;
pub mod favorites;
pub mod fleet;
pub mod format;
pub mod guardrails;
pub mod oauth;
pub mod overrides;
pub mod pricing;
pub mod process_identity;
pub mod project;
pub mod review;
pub mod reviewed;
pub mod routing;
pub mod sessions;
pub mod transcript;
pub mod types;
pub mod windows;

pub use antigravity_sessions::idle_sessions as antigravity_idle_sessions;
pub use claude_registry_lifecycle::{
    claude_stale_registry_candidate, cleanup_claude_stale_registry_entry,
    ClaudeRegistryCleanupOutcome, ClaudeRegistryLifecycleError, ClaudeStaleRegistryCandidate,
};
pub use conductor::{
    fleet_conductor_session_count, fleet_is_large, fleet_session_count, group_project_sessions,
    host_auto_collapsed, host_conductor_session_count, host_ident, host_key, host_key_is_local,
    host_session_count, host_summary, model_effort_label, next_waiting, session_attr_line,
    session_attrs, session_glyph, session_identity_key, session_identity_key_with_account_id,
    session_key, split_host_key, state_word, waiting_sessions, ConductorSession, SessionAttrs,
    WaitingTarget, GLYPH_IDLE, GLYPH_WAITING, GLYPH_WORKING,
};
pub use favorites::{Favorite, FavoriteKind, Favorites};
pub use fleet::{
    build_fleet_tree, fold_inventory, reconcile_connected_hosts, sessions_of_account,
    AccountSession, AgentInventoryStatus, AgentSession, FleetTree, HostAvailability, HostNode,
    HostSessions, ProjectNode, RegisteredHost, RemoteHost,
};
pub use format::{
    binding_window, context_fill, context_window, format_cost, format_relative, format_reset,
    format_tokens, heat_fill, heat_pct_label_with_provenance, model_family, HeatLevel,
};
pub use guardrails::{
    failed_toast, guardrail_target, kill_confirm_message, no_remote_connection_toast,
    pid_signalable, remote_unsupported_toast, sent_toast, session_label, stop_all_confirm_message,
    stop_all_summary_toast, unsignalable_toast, GuardrailSignal, GuardrailTarget,
};
pub use oauth::{apply_oauth_usage, parse_oauth_usage, OauthUsage, OauthWindow};
pub use overrides::{set_label_override, AccountOverride, AccountOverrides};
pub use pricing::{ModelPrice, PricingSource, PricingTable};
pub use process_identity::{
    current_process_fingerprint, local_process_signalling_supported, probe_registered_process,
    send_verified_process_signal, ProcessPresence, ProcessProbe, ProcessSignalError,
};
pub use project::{resolve_project, ResolvedProject};
pub use review::{git_commit_all_cmd, git_diff_cmd, render_review_markdown, WorkingChanges};
pub use reviewed::{ReviewedSessions, REVIEWED_LIMIT};
pub use routing::pick_freest_checked;
pub use routing::{is_over_budget, pick_freest, rank_by_freeness, OVER_BUDGET_HEAT};
pub use transcript::{
    format_transcript_markdown, parse_task_state, parse_transcript, LoadedTranscript, ToolCall,
    TranscriptTurn, TurnRole, TurnUsage,
};
pub use types::{
    Account, AccountStatus, AccountUsage, CockpitSnapshot, Provider, ScanHealth, SessionSnapshot,
    SessionState, TaskItem, TaskState, TaskStatus, UsageEntry, UsageProvenance, WindowTotals,
};
pub use windows::{
    build_account_usage, window_5h, window_week, with_idle_sessions, with_sessions,
    DEFAULT_BUDGET_5H, DEFAULT_BUDGET_WEEK,
};

use std::path::Path;

use chrono::{DateTime, Duration, Utc};

/// Process-local, bounded transcript parse caches shared across reconcile
/// cycles. It stores only the same structured task titles already projected on
/// session snapshots, never credentials or conversational message text.
#[derive(Clone, Debug, Default)]
pub struct TranscriptScanCache {
    task_states: transcript::TaskStateCache,
    codex_rollouts: codex_sessions::RolloutCache,
}

/// How far back dormant-session discovery looks.
///
/// Not a limit of `claude --resume`, `codex resume`, or `agy --conversation` —
/// provider state lives far longer than this. It is a *usefulness* bound:
/// picking a conversation back up is something you do within days, and by then
/// the working tree it refers to has usually moved on. Older ones would be list
/// noise, so they stay out.
pub const IDLE_MAX_AGE: Duration = Duration::days(7);

/// Upper bound on dormant sessions surfaced per account, most-recent first.
/// Discovery walks a history that grows without limit, so the cost of a refresh
/// and the length of the session list must not grow with it.
pub const IDLE_SESSION_LIMIT: usize = 50;

/// Build a full cockpit snapshot from disk: discover Claude + Codex accounts, parse
/// their transcripts within the widest (week) window, and aggregate per-account
/// usage / cost / heat.
///
/// `codex_home` is the caller-resolved `$CODEX_HOME`, or `~/.codex` when the
/// variable is unset. A non-default value is scanned alongside the default root and
/// retained as the account pin. `claude_config_dir_env` is handled equivalently.
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
    build_snapshot_with_cache(
        home,
        codex_home,
        claude_config_dir_env,
        now,
        budget_5h,
        budget_week,
        pricing,
        &mut TranscriptScanCache::default(),
    )
}

pub fn build_snapshot_with_cache(
    home: &Path,
    codex_home: &Path,
    claude_config_dir_env: Option<&str>,
    now: DateTime<Utc>,
    budget_5h: u64,
    budget_week: u64,
    pricing: &PricingTable,
    transcript_cache: &mut TranscriptScanCache,
) -> CockpitSnapshot {
    let since = now - window_week();
    let mut accounts = Vec::new();
    // Reasons the scan degraded (a present-but-unreadable config/dir), collected so an
    // empty/short accounts list is reported as "load failed" rather than "genuinely
    // empty" — and excluded from freest-account routing. Messages are English
    // technical detail for logs; the UI shows its own plain message, not these strings.
    let mut degraded: Vec<String> = Vec::new();
    let claude_discovery = claude::discover_accounts_with_health(home, claude_config_dir_env);
    degraded.extend(claude_discovery.issues);
    for account in claude_discovery.accounts {
        // The walk reports its own I/O errors now (permission on any subdir, not just
        // the projects/ root) — a silently-truncated scan reads as "never used" and
        // would win freest-account routing.
        let (entries, io_error) = claude::usage_for_account(&account, since);
        if io_error {
            degraded.push(format!("{}: usage history unreadable", account.label));
        }
        // One scan: live and dormant are decided by the same pid probe, so a
        // session cannot show up as both because it exited between two passes.
        let scan = sessions::scan_sessions_with_cache(
            &account.config_dir,
            now,
            IDLE_MAX_AGE,
            IDLE_SESSION_LIMIT,
            &mut transcript_cache.task_states,
        );
        // Route + identity, from the one function the daemon also uses.
        let stamp = |mut s: SessionSnapshot| {
            account.stamp(&mut s);
            s
        };
        let live: Vec<SessionSnapshot> = scan.live.into_iter().map(stamp).collect();
        // Explicit user budgets win; otherwise estimate from the plan tier so
        // Enterprise/Team accounts aren't shown falsely maxed.
        let (plan_5h, plan_week) = windows::plan_budgets(account.plan_tier.as_deref());
        let b5h = if budget_5h > 0 { budget_5h } else { plan_5h };
        let bwk = if budget_week > 0 {
            budget_week
        } else {
            plan_week
        };
        // Dormant conversations of this account: not running, but resumable.
        // Stamped like the live ones, so adopting one re-enters the subscription
        // it belongs to rather than whichever account happens to be default.
        let idle: Vec<SessionSnapshot> = scan.idle.into_iter().map(stamp).collect();
        let usage = build_account_usage(account, entries, now, b5h, bwk, pricing);
        accounts.push(windows::with_idle_sessions(
            windows::with_sessions(usage, live),
            idle,
        ));
    }
    let default_codex_home = home.join(".codex");
    let pinned_codex_home = (codex_home != default_codex_home.as_path()).then_some(codex_home);
    let codex_discovery = codex::discover_account_roots(home, pinned_codex_home);
    degraded.extend(codex_discovery.issues);
    for account in codex_discovery.accounts {
        let (entries, io_error) = codex::usage_for_account(&account, since);
        if io_error {
            degraded.push(format!("{}: usage history unreadable", account.label));
        }
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
        // Codex agent-sessions (Step 8 parity): transcript-inferred, no
        // registry/pid (see `codex_sessions`). Attached so they flow into the
        // unified Agent-Inventory exactly like Claude's — one walk, so the live
        // window classifies each rollout once.
        let scan = codex_sessions::scan_sessions_with_cache(
            &account.config_dir,
            now,
            IDLE_MAX_AGE,
            IDLE_SESSION_LIMIT,
            &mut transcript_cache.codex_rollouts,
        );
        let stamp = |mut s: SessionSnapshot| {
            account.stamp(&mut s);
            s
        };
        let live: Vec<SessionSnapshot> = scan.live.into_iter().map(stamp).collect();
        let idle: Vec<SessionSnapshot> = scan.idle.into_iter().map(stamp).collect();
        let usage = build_account_usage(account, entries, now, b5h, bwk, pricing);
        accounts.push(windows::with_idle_sessions(
            windows::with_sessions(usage, live),
            idle,
        ));
    }

    let health = if degraded.is_empty() {
        ScanHealth::Loaded
    } else {
        ScanHealth::Degraded(degraded.join("; "))
    };
    CockpitSnapshot {
        accounts,
        generated_at: now,
        health,
    }
}

/// Cached live-session scan for one Claude account, used by remote daemons.
pub fn live_claude_sessions_with_cache(
    config_dir: &Path,
    now: DateTime<Utc>,
    transcript_cache: &mut TranscriptScanCache,
) -> Vec<SessionSnapshot> {
    sessions::scan_sessions_with_cache(
        config_dir,
        now,
        Duration::zero(),
        0,
        &mut transcript_cache.task_states,
    )
    .live
}

/// Cached live-session scan for one Codex account, used by remote daemons.
pub fn live_codex_sessions_with_cache(
    config_dir: &Path,
    now: DateTime<Utc>,
    transcript_cache: &mut TranscriptScanCache,
) -> Vec<SessionSnapshot> {
    codex_sessions::scan_sessions_with_cache(
        config_dir,
        now,
        Duration::zero(),
        0,
        &mut transcript_cache.codex_rollouts,
    )
    .live
}

#[cfg(test)]
mod build_snapshot_health_tests {
    use super::*;
    use chrono::Utc;
    use std::fs;

    /// A present-but-unreadable `auth.json` makes codex discovery return no account —
    /// identical in shape to "Codex was never set up". The snapshot must report this
    /// as *degraded* so the UI can say "couldn't read your account" (and offer a
    /// retry) instead of the misleading "no accounts".
    #[test]
    fn a_malformed_codex_auth_json_degrades_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let codex_home = tmp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(codex_home.join("auth.json"), "{ this is not valid json").unwrap();

        let snap = build_snapshot(
            &home,
            &codex_home,
            None,
            Utc::now(),
            0,
            0,
            &PricingTable::default(),
        );
        assert!(
            matches!(snap.health, ScanHealth::Degraded(_)),
            "a present-but-unreadable codex auth.json must degrade, not read as empty: {:?}",
            snap.health,
        );
    }

    #[test]
    fn a_malformed_claude_identity_degrades_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let codex_home = home.join(".codex");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/.claude.json"), "{not valid json").unwrap();

        let snap = build_snapshot(
            &home,
            &codex_home,
            None,
            Utc::now(),
            0,
            0,
            &PricingTable::default(),
        );
        assert!(
            matches!(snap.health, ScanHealth::Degraded(_)),
            "a malformed Claude identity must degrade, not invent an account: {:?}",
            snap.health,
        );
        assert!(snap.accounts.is_empty());
    }

    /// A clean setup with genuinely no accounts is authoritative — an empty list that
    /// the UI may present as a real "no accounts" (with a sign-in prompt), not a
    /// failure.
    #[test]
    fn a_clean_empty_setup_is_loaded_not_degraded() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let codex_home = tmp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&codex_home).unwrap();

        let snap = build_snapshot(
            &home,
            &codex_home,
            None,
            Utc::now(),
            0,
            0,
            &PricingTable::default(),
        );
        assert_eq!(
            snap.health,
            ScanHealth::Loaded,
            "a clean, genuinely-empty setup is authoritative-empty",
        );
    }
}

#[cfg(test)]
#[path = "snapshot_platform_tests.rs"]
mod snapshot_platform_tests;
