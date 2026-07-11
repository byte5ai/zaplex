//! Core cockpit data types — pure, serde-friendly, no I/O and no secrets.
//!
//! Privacy invariant: these types carry only **token counts and account metadata**,
//! never token strings, transcript content, or any credential material.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The LLM CLI providers the cockpit understands.
///
/// A minimal enum owned by this (pure) crate; the app's richer `CLIAgent` maps onto
/// it at the wiring layer. Increment 1 covers Claude Code + Codex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

/// A discovered account / subscription. Metadata only — never tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub provider: Provider,
    /// Stable key derived from the provider + config dir, e.g. `claude:default`,
    /// `claude:work`, `codex:default`. Stable across restarts for pinning later.
    pub key: String,
    /// The config directory this account was discovered from.
    pub config_dir: PathBuf,
    /// Human label (email/org/plan-derived; falls back to the dir name).
    pub label: String,
    pub email: Option<String>,
    pub org: Option<String>,
    pub role: Option<String>,
    /// Plan tier label, e.g. "Max 20x", "Max", "Pro" (best-effort, provider-specific).
    pub plan_tier: Option<String>,
    /// Whether this is the provider's default config dir (`~/.claude`, `~/.codex`).
    pub is_default: bool,
}

/// One usage record extracted from a transcript line (one assistant turn/message).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageEntry {
    pub ts: DateTime<Utc>,
    pub provider: Provider,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_create: u64,
    pub cache_read: u64,
    /// Codex reasoning output tokens (billed as output); 0 for Claude.
    pub reasoning: u64,
}

/// Aggregated token + cost totals over a time window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowTotals {
    pub input: u64,
    pub output: u64,
    pub cache_create: u64,
    pub cache_read: u64,
    pub reasoning: u64,
    /// Load signal: `input + output + cache_create + reasoning` — excludes the cheap,
    /// high-volume cache *reads* so heat/"launch-on-freest" reflect real work.
    pub work: u64,
    /// All billable tokens: `work + cache_read`.
    pub total: u64,
    pub cost_usd: f64,
    /// Number of assistant messages/turns counted.
    pub messages: u64,
}

impl WindowTotals {
    /// Fold one usage entry into the running totals, adding its cost via `pricing`.
    pub fn add(&mut self, e: &UsageEntry, pricing: &crate::pricing::PricingTable) {
        self.input += e.input;
        self.output += e.output;
        self.cache_create += e.cache_create;
        self.cache_read += e.cache_read;
        self.reasoning += e.reasoning;
        self.work += e.input + e.output + e.cache_create + e.reasoning;
        self.total += e.input + e.output + e.cache_create + e.cache_read + e.reasoning;
        self.cost_usd += pricing.cost_for(
            &e.model,
            e.input,
            e.output,
            e.cache_create,
            e.cache_read,
            e.reasoning,
        );
        self.messages += 1;
    }
}

/// Live-session state, waiting-first semantics (see `sessions.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Registry-reported busy: the agent is working right now.
    Active,
    /// The assistant's last turn ended — the session is waiting for YOU.
    Waiting,
    /// Mid tool-run or a live background job: working, hands off.
    Monitor,
    /// A transcript exists but the conversation has **no live PTY / registry
    /// entry** — dormant, resumable. Idle is never "needs me" and sorts after
    /// the live states (Waiting/Active/Monitor).
    Idle,
}

/// One agent-session snapshot (registry-backed + transcript-joined for live
/// Claude Code sessions; the same shape carries dormant [`SessionState::Idle`]
/// sessions once transcript-only discovery lands).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub cwd: String,
    /// The session's registry name (native `--name`), often empty.
    pub name: String,
    pub state: SessionState,
    /// Which CLI provider owns this session.
    pub provider: Provider,
    /// Model of the latest assistant turn (may be empty).
    pub model: String,
    /// Reasoning effort. Not recorded in the transcript, so `None` at discovery
    /// time; populated at launch time later. `None` = honestly unknown.
    pub effort: Option<String>,
    /// Context-window fill of the latest assistant turn.
    pub ctx_tokens: u64,
    /// Git-root of `cwd` (or `cwd` itself when not inside a repo). The
    /// project-grouping key for the Agent-Inventory tree.
    pub project_root: String,
    /// Human repo label — origin-url basename, else the root's dir basename.
    pub project_name: String,
    pub last_activity: DateTime<Utc>,
    pub pid: u32,
}

/// Coarse account activity derived from its live sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    /// At least one session is actively working.
    Working,
    /// Sessions exist, none busy.
    Live,
    /// No live sessions.
    Offline,
}

/// Where an account's heat/reset numbers come from.
///
/// The cockpit prefers the provider's **real** rate-limit position (OAuth usage
/// endpoint, C3b) and falls back to the local transcript-based **estimate**
/// whenever the real number is unavailable — honest degradation, visibly marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageProvenance {
    /// Real utilization reported by the provider for this account.
    Real,
    /// Derived locally from transcript token counts vs. a budget guess.
    #[default]
    Estimate,
}

/// Per-account usage across the cockpit's windows, plus derived reset times + heat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountUsage {
    pub account: Account,
    /// Current rolling 5-hour block.
    pub block5h: WindowTotals,
    /// Current calendar day (UTC in Increment 1).
    pub today: WindowTotals,
    /// Current rolling 7-day block.
    pub week: WindowTotals,
    /// When the current 5h block resets (block start + 5h), if a block is active.
    pub reset5h: Option<DateTime<Utc>>,
    /// When the current 7d block resets, if a block is active.
    pub reset_week: Option<DateTime<Utc>>,
    /// `block5h.work / budget_5h`, clamped at 0; may exceed 1.0 (over budget).
    pub heat: f64,
    /// `week.work / budget_week` — the slower weekly budget's heat; same
    /// semantics as `heat` (may exceed 1.0).
    pub heat_week: f64,
    /// 7-day **Opus** sublimit utilization (Max plans), same fraction scale as
    /// `heat`. `Some` only for real OAuth accounts whose plan reports it — often
    /// the binding constraint for Opus-heavy users; `None` for estimates.
    #[serde(default)]
    pub heat_opus: Option<f64>,
    /// 7-day **Sonnet** sublimit utilization, same scale as `heat`. `Some` only
    /// for real OAuth accounts whose plan reports it; `None` otherwise.
    #[serde(default)]
    pub heat_sonnet: Option<f64>,
    /// Live sessions (Claude Code registry), waiting-first. Empty for
    /// providers without a session registry (Codex, for now).
    pub sessions: Vec<SessionSnapshot>,
    /// Coarse activity status derived from `sessions`.
    pub status: AccountStatus,
    /// Whether `heat`/`heat_week`/resets are the provider's real numbers or the
    /// local estimate (see [`UsageProvenance`]). Token/cost totals are always
    /// transcript-derived — they measure spend, not rate-limit position.
    #[serde(default)]
    pub provenance: UsageProvenance,
}

/// A full cockpit snapshot: every discovered account with its usage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CockpitSnapshot {
    pub accounts: Vec<AccountUsage>,
    pub generated_at: DateTime<Utc>,
}
