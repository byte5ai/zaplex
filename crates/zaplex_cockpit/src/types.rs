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
}

/// A full cockpit snapshot: every discovered account with its usage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CockpitSnapshot {
    pub accounts: Vec<AccountUsage>,
    pub generated_at: DateTime<Utc>,
}
