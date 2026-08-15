//! Core cockpit data types — pure, serde-friendly, no I/O and no secrets.
//!
//! Privacy invariant: these types never carry token strings or credential
//! material. Session snapshots may carry the task titles that an external agent
//! deliberately emitted through its structured task/plan tool.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A JSON-safe sentinel for an aggregated window containing unpriced usage.
/// It stays negative under ordinary fleet sums; real token costs cannot be
/// negative, and [`crate::format::format_cost`] renders every negative value as
/// `unpriced`. The explicit [`WindowTotals::has_unpriced_usage`] flag remains
/// the semantic source of truth.
pub(crate) const UNPRICED_COST_USD: f64 = -1.0e300;

/// The LLM CLI providers the cockpit understands.
///
/// A minimal enum owned by this (pure) crate; the app's richer `CLIAgent` maps onto
/// it at the wiring layer. Account/usage discovery covers Claude Code + Codex;
/// session discovery additionally covers Antigravity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Claude,
    Codex,
    Antigravity,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Antigravity => "antigravity",
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

impl Account {
    /// The config dir to **pin** when resuming/forking a session under this
    /// account (`CODEX_HOME` / `CLAUDE_CONFIG_DIR`), or `None` for the default
    /// account (which needs no pin). Single source so the local snapshot builder
    /// and the remote daemon stamp [`SessionSnapshot::config_dir`] identically —
    /// otherwise a remote resume can't route to a plexed subscription.
    pub fn config_dir_pin(&self) -> Option<String> {
        (!self.is_default).then(|| self.config_dir.to_string_lossy().into_owned())
    }

    /// Stamp a session discovered under this account with the two things it
    /// cannot know about itself: how to **route** back to the account
    /// ([`Self::config_dir_pin`]) and **whose** account it is (the email — the
    /// only identity that means the same on another host).
    ///
    /// One function, because both the local snapshot builder and the daemon do
    /// this to their own hosts' sessions. Two copies would eventually stamp
    /// differently, and the client's join would quietly stop matching.
    pub fn stamp(&self, session: &mut SessionSnapshot) {
        session.config_dir = self.config_dir_pin();
        session.account_email = self.email.clone();
    }
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
    /// The session this turn belongs to — the same id [`SessionSnapshot`]
    /// carries, so spend can be attributed per session rather than only per
    /// account. Empty when the transcript does not name one (a Codex rollout
    /// with no `session_meta`); such turns still count towards the account, they
    /// just cannot be pinned to a row.
    #[serde(default)]
    pub session_id: String,
}

/// Aggregated token + cost totals over a time window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowTotals {
    pub input: u64,
    pub output: u64,
    pub cache_create: u64,
    pub cache_read: u64,
    pub reasoning: u64,
    /// Load signal: `input + output + cache_create` — excludes the cheap,
    /// high-volume cache *reads* so heat/"launch-on-freest" reflect real work.
    /// Codex reasoning is already included in `output` and is retained only as
    /// a separately displayable breakdown.
    pub work: u64,
    /// All billable tokens: `work + cache_read`.
    pub total: u64,
    /// `true` if at least one turn's model had no static price. In that case
    /// `cost_usd` is a JSON-safe negative sentinel, which
    /// [`crate::format::format_cost`] renders as `unpriced` instead of
    /// pretending the unknown spend is `$0.00`.
    #[serde(default)]
    pub has_unpriced_usage: bool,
    pub cost_usd: f64,
    /// Number of assistant messages/turns counted.
    pub messages: u64,
}

impl WindowTotals {
    /// Fold one usage entry into the running totals, adding its cost via `pricing`.
    pub fn add(&mut self, e: &UsageEntry, pricing: &crate::pricing::PricingTable) {
        self.input = self.input.saturating_add(e.input);
        self.output = self.output.saturating_add(e.output);
        self.cache_create = self.cache_create.saturating_add(e.cache_create);
        self.cache_read = self.cache_read.saturating_add(e.cache_read);
        self.reasoning = self.reasoning.saturating_add(e.reasoning);
        let work = e
            .input
            .saturating_add(e.output)
            .saturating_add(e.cache_create);
        let total = work.saturating_add(e.cache_read);
        self.work = self.work.saturating_add(work);
        self.total = self.total.saturating_add(total);
        match pricing.cost_for(&e.model, e.input, e.output, e.cache_create, e.cache_read) {
            Some(cost) if !self.has_unpriced_usage => self.cost_usd += cost,
            Some(_) => {}
            None => {
                self.has_unpriced_usage = true;
                self.cost_usd = UNPRICED_COST_USD;
            }
        }
        self.messages = self.messages.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;

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

/// Provider-neutral state of one task emitted by an external agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// One ordered row in an external agent's structured task state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskItem {
    /// Provider-local stable identity. Claude uses its task id; Codex uses the
    /// row's stable position in the latest full `update_plan` replacement.
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
}

/// The latest structured task state emitted by a session.
///
/// `Some` with an empty `tasks` list means the provider explicitly emitted an
/// empty state; `None` means no usable task state was observed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskState {
    pub tasks: Vec<TaskItem>,
}

/// One agent-session snapshot (registry-backed + transcript-joined for live
/// Claude Code sessions). The same shape carries dormant [`SessionState::Idle`]
/// sessions, which `sessions::scan_sessions` discovers from the transcript that
/// outlives the process.
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
    /// Git-root of `cwd` (or `cwd` itself when not inside a repo) — **this
    /// session's own working tree**. What acts on the files the session sees
    /// (review, a launch scoped to it) must use this, not `repo_root`: for a
    /// linked worktree the two differ, and the repo's tree holds other work.
    pub project_root: String,
    /// Working-tree root of the **repo** `project_root` belongs to — the main
    /// checkout for a linked worktree, `project_root` itself otherwise.
    ///
    /// The inventory's grouping key: several worktrees of one repo are one
    /// project with several sessions, not several projects that merely share a
    /// name. Empty from a producer that predates the field; grouping then falls
    /// back to `project_root`, i.e. to the old per-worktree split — the honest
    /// degradation, not a wrong grouping.
    #[serde(default)]
    pub repo_root: String,
    /// Human repo label — origin-url basename, else the root's dir basename.
    pub project_name: String,
    /// Git branch checked out in this session's working tree (`.git/HEAD` →
    /// `refs/heads/<branch>`), e.g. `main`, `rc/master-plan`. `None` when
    /// detached, unknown, or not a repo. The **primary session-identity
    /// signal** (redesign §2.2): it disambiguates several parallel agents on
    /// one repo, which the model alone cannot.
    #[serde(default)]
    pub branch: Option<String>,
    /// Linked-worktree name when `cwd` sits in a git worktree distinct from the
    /// main checkout (the `worktrees/<name>` leaf of the gitdir), e.g.
    /// `rc-master-plan`. `None` for the primary worktree / non-repo. Pairs with
    /// `branch` to identify a session (redesign §2.2).
    #[serde(default)]
    pub worktree: Option<String>,
    /// The provider config directory this session's account lives under
    /// (`~/.codex` / a plexed `CODEX_HOME`, `~/.claude` / `CLAUDE_CONFIG_DIR`).
    /// Carried so a **remote** resume can pin the right subscription — over the
    /// wire this is the *host's* path, replayed verbatim in the daemon's
    /// `startup_command`. `None` for the default account (no pin needed) or when
    /// the producer didn't record it.
    ///
    /// **Routing, not identity.** It names a directory on the host that produced
    /// it, so it says nothing about *which subscription* this is: two hosts spell
    /// the same account differently, and a default account carries no pin at all.
    /// To ask "whose account is this?", use `account_email`.
    #[serde(default)]
    pub config_dir: Option<String>,
    /// The email of the account this session runs under — the one thing about a
    /// session that means the same on every host, and therefore the only sound
    /// way to tell that a remote session belongs to an account discovered here.
    ///
    /// Paired with `provider` it is the join key: one address can own both a
    /// Claude and a Codex subscription. `None` when the producer could not
    /// determine it (no OAuth identity on that host, or an older daemon that
    /// does not send it) — such a session stays visible in the host tree and
    /// simply joins no account, rather than being guessed onto one.
    #[serde(default)]
    pub account_email: Option<String>,
    /// Opaque identity of the exact operating-system process observed during
    /// discovery. A pid can be recycled after that scan; Stop/Kill is safe only
    /// when an immediate re-probe yields this same fingerprint. `None` means
    /// the process could not be proven and must remain unsignalable.
    #[serde(default)]
    pub process_fingerprint: Option<String>,
    /// Generation-checked daemon PTY currently or historically associated with
    /// this agent. Present only after capability negotiation.
    #[serde(default)]
    pub pty_session_id: Option<String>,
    #[serde(default)]
    pub pty_session_generation: Option<u64>,
    /// Whether this is the single attachable foreground agent for that PTY.
    #[serde(default)]
    pub pty_foreground: bool,
    /// Structured provider-neutral task state reconstructed from the session's
    /// transcript. Absent for sessions that never emitted a supported task
    /// tool, and for snapshots from older producers.
    #[serde(default)]
    pub task_state: Option<TaskState>,
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
    /// Current calendar day on the **local** clock (see `windows::today_totals`).
    pub today: WindowTotals,
    /// `today`, split by the session that spent it — what the session table's
    /// "today $" column reads, keyed by [`SessionSnapshot::session_id`].
    ///
    /// Folded from the same entries under the same day rule as `today`: every
    /// turn is counted once, on exactly one side, so the rows account for the
    /// figure above them rather than being a second estimate of it. The token
    /// counters therefore sum to `today` exactly. `cost_usd` does not: grouping
    /// the same summands per session adds them in a different order, and f64
    /// addition is not associative. The drift is far below a cent — the test
    /// pins it under 1e-9 — but it is a tolerance, not a rounding step, and not
    /// a bound anyone has proved.
    ///
    /// The empty key collects turns whose transcript names no session: counted
    /// for the account, just not attributable to a row.
    #[serde(default)]
    pub today_by_session: BTreeMap<String, WindowTotals>,
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
    /// **Live** sessions — a process is provably there (Claude: a pid-alive
    /// registry entry; Codex: a rollout touched inside its live window),
    /// waiting-first. The surfaces that mean "running work" — the Conductor
    /// tree, `status`, the account card's count — read this field alone, so a
    /// finished conversation cannot be counted as work in progress.
    ///
    /// Producers must keep [`SessionState::Idle`] out of here and put dormant
    /// sessions in `idle_sessions`; today's do, by construction (neither
    /// provider's state function can return `Idle`). That is a contract, not an
    /// enforced invariant — `windows::with_sessions` therefore derives `status`
    /// in a way that stays right even if an `Idle` ever slips in.
    pub sessions: Vec<SessionSnapshot>,
    /// **Dormant** sessions: the CLI process is gone, but the transcript
    /// survives, so `--resume` can pick the conversation back up. Bounded and
    /// most-recent-first. Disjoint from `sessions` because a single scan
    /// (`sessions::scan_sessions`) classifies each session once — filling these
    /// two fields from two separate scans would let one that exits in between
    /// land in both. Surfaced by the session table's Idle filter; deliberately
    /// kept out of the Conductor, which lists live work.
    #[serde(default)]
    pub idle_sessions: Vec<SessionSnapshot>,
    /// Coarse activity status derived from `sessions` — dormant sessions do not
    /// make an account "live".
    pub status: AccountStatus,
    /// Whether `heat`/`heat_week`/resets are the provider's real numbers or the
    /// local estimate (see [`UsageProvenance`]). Token/cost totals are always
    /// transcript-derived — they measure spend, not rate-limit position.
    #[serde(default)]
    pub provenance: UsageProvenance,
}

/// Load health of a [`CockpitSnapshot`]. Lets a reader tell "no accounts because
/// the scan has not run / failed" apart from "no accounts because there genuinely
/// are none" — an empty `accounts` list is otherwise ambiguous, which both misleads
/// the empty-state UI and lets the launcher's freest-account pick silently choose an
/// account whose usage merely failed to load (it looks maximally free).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScanHealth {
    /// No disk scan has completed yet: `accounts` is "not loaded", not "empty".
    #[default]
    Pending,
    /// The scan completed and `accounts` is authoritative.
    Loaded,
    /// The scan completed but hit a present-but-unreadable config/dir, so
    /// `accounts` may be missing real accounts or usage. Carries a human reason.
    Degraded(String),
}

impl ScanHealth {
    /// The snapshot is authoritative — safe to route from, and an empty `accounts`
    /// then means a genuine "no accounts".
    pub fn is_loaded(&self) -> bool {
        matches!(self, ScanHealth::Loaded)
    }
}

/// A full cockpit snapshot: every discovered account with its usage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CockpitSnapshot {
    pub accounts: Vec<AccountUsage>,
    pub generated_at: DateTime<Utc>,
    /// Load health — distinguishes "still loading" / "scan failed" from "genuinely
    /// empty". `#[serde(default)]` = `Pending` for snapshots written before this field.
    #[serde(default)]
    pub health: ScanHealth,
}
