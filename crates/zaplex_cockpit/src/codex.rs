//! Codex account discovery + session (rollout) usage parsing.
//!
//! Net-new (no `claudeplex` prior art); the exact on-disk schema is only partly
//! confirmed (design doc §10), so parsing is deliberately **defensive**: it searches
//! each JSONL line for a token-usage object rather than assuming a fixed path.
//!
//! Privacy: reads local `auth.json` account metadata, including the auth form,
//! and decodes the **unverified** `id_token` JWT payload for its `email` and
//! plan claims. Raw token and API-key strings are never returned, persisted,
//! logged, or sent over the network.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::transcript::{file_fingerprint, FileFingerprint};
use crate::types::{Account, AccountUsage, Provider, UsageEntry, UsageProvenance};

/// Result of account-root discovery before rollout/session scanning.
#[derive(Debug, Default)]
pub struct AccountDiscovery {
    pub accounts: Vec<Account>,
    pub issues: Vec<String>,
}

/// Recursively find the first sub-value under `key` anywhere in `v`.
fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|val| find(val, key))
        }
        Value::Array(arr) => arr.iter().find_map(|val| find(val, key)),
        _ => None,
    }
}

/// Decode the (unverified) payload of a JWT and return its claims object. Never used
/// for auth — only to read the display `email` and plan claims. Returns `None` on
/// any malformation.
fn jwt_payload(token: &str) -> Option<Value> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The id_token claim namespace OpenAI puts its account claims under.
const OPENAI_AUTH_CLAIMS: &str = "https://api.openai.com/auth";

/// The raw `chatgpt_plan_type` claim: namespaced under [`OPENAI_AUTH_CLAIMS`] in
/// current tokens, accepted top-level as well.
fn plan_type_claim(claims: &Value) -> Option<&str> {
    claims
        .get(OPENAI_AUTH_CLAIMS)
        .and_then(|auth| auth.get("chatgpt_plan_type"))
        .or_else(|| claims.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
}

/// Readable label for a raw ChatGPT plan type, e.g. `prolite` -> "Pro Lite".
/// Unknown values are title-cased (`free_workspace` -> "Free Workspace") rather
/// than dropped, so a new plan still tells two workspaces apart.
pub(crate) fn plan_label(raw: &str) -> String {
    let plan = raw.trim().to_ascii_lowercase();
    let known = match plan.as_str() {
        "pro" => Some("Pro"),
        "plus" => Some("Plus"),
        "prolite" => Some("Pro Lite"),
        "team" => Some("Team"),
        "enterprise" => Some("Enterprise"),
        "edu" => Some("Edu"),
        "free" => Some("Free"),
        business
            if business.starts_with("business") || business.starts_with("self_serve_business") =>
        {
            Some("Business")
        }
        _ => None,
    };
    if let Some(label) = known {
        return label.to_string();
    }
    plan.split(['_', '-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn account_key(codex_home: &Path, home: &Path, is_default: bool) -> String {
    crate::account_key::stable_account_key(Provider::Codex, codex_home, home, is_default)
}

fn account_from_root(
    codex_home: &Path,
    home: &Path,
    is_default: bool,
) -> Result<Option<(Account, Option<String>)>, String> {
    let auth_path = codex_home.join("auth.json");
    let raw = match fs::read_to_string(&auth_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Codex account sign-in file is unreadable".to_string()),
    };
    let auth = serde_json::from_str::<Value>(&raw)
        .map_err(|_| "Codex account sign-in file is malformed".to_string())?;

    let auth_mode = auth
        .get("auth_mode")
        .and_then(|x| x.as_str())
        .filter(|mode| !mode.is_empty())
        .map(|s| s.to_string());

    // Email from the id_token JWT payload (best-effort; token itself is never stored).
    let id_token = auth
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(Value::as_str);
    let claims = id_token.and_then(jwt_payload);
    let email = claims.as_ref().and_then(|claims| {
        claims
            .get("email")
            .and_then(|e| e.as_str())
            .map(|s| s.to_string())
    });
    let plan_tier = claims
        .as_ref()
        .and_then(plan_type_claim)
        .map(plan_label)
        .filter(|label| !label.is_empty());
    let account_id = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);

    let has_legacy_api_key = auth
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|key| !key.trim().is_empty());
    if auth_mode.as_deref() == Some("apikey") {
        return Ok(None);
    }

    let has_subscription_identity =
        auth_mode.as_deref() == Some("chatgpt") || account_id.is_some() || email.is_some();
    if !has_subscription_identity && has_legacy_api_key {
        return Ok(None);
    }
    if !has_subscription_identity {
        return Err("Codex account sign-in file is malformed".to_string());
    }

    let label = email
        .clone()
        .or_else(|| auth_mode.clone())
        .unwrap_or_else(|| "codex".to_string());

    Ok(Some((
        Account {
            provider: Provider::Codex,
            key: account_key(codex_home, home, is_default),
            config_dir: codex_home.to_path_buf(),
            label,
            provider_account_id: account_id.clone(),
            email,
            org: None,
            role: None,
            // Provider ≠ plan (WS4 S2): `auth_mode` ("chatgpt" / "apikey") is *how* you
            // authenticate, not a subscription plan. The plan comes from the
            // id_token's `chatgpt_plan_type` claim; without it it stays unknown.
            plan_tier,
            is_default,
        },
        account_id,
    )))
}

/// Discover Codex accounts from the documented default `~/.codex` root, the
/// sorted `~/.codex-*` siblings (each a `CODEX_HOME` of its own), and an
/// optional root pinned by `$CODEX_HOME`.
///
/// Existing roots are canonicalized, then aliases are deduplicated by canonical
/// path or the stable `account_id` in `auth.json`. Codex itself still exposes one
/// account per root; this function merely unions the deterministic roots. Two
/// workspaces of one login carry distinct `account_id`s and stay two accounts.
pub fn discover_account_roots(home: &Path, codex_home_env: Option<&Path>) -> AccountDiscovery {
    let default_root = home.join(".codex");
    let mut candidates: Vec<(PathBuf, bool, bool)> = vec![(default_root.clone(), true, false)];
    let mut issues = Vec::new();
    match fs::read_dir(home) {
        Ok(read) => {
            let mut siblings = Vec::new();
            for entry in read {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        issues.push("Codex account directory entry is unreadable".to_string());
                        continue;
                    }
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.starts_with(".codex-")
                    || crate::claude::is_excluded_sibling(".codex-", name)
                {
                    continue;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() || kind.is_symlink() => siblings.push(entry.path()),
                    Ok(_) => {}
                    Err(_) => issues.push("Codex account root is unreadable".to_string()),
                }
            }
            siblings.sort();
            candidates.extend(siblings.into_iter().map(|path| (path, false, false)));
        }
        Err(_) if !matches!(home.try_exists(), Ok(false)) => {
            issues.push("Codex accounts: home directory unreadable".to_string());
        }
        Err(_) => {}
    }
    if let Some(root) = codex_home_env {
        candidates.push((root.to_path_buf(), root == default_root.as_path(), true));
    }

    let mut accounts = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut seen_identities = HashSet::new();
    let mut seen_keys = HashSet::new();
    for (root, is_default, is_pinned) in candidates {
        let canonical_root = match fs::canonicalize(&root) {
            Ok(canonical_root) => canonical_root,
            Err(_) if fs::symlink_metadata(&root).is_ok() => {
                issues.push("Codex account root is unreadable".to_string());
                continue;
            }
            Err(_) if matches!(root.try_exists(), Ok(false)) => root.clone(),
            Err(_) => {
                issues.push("Codex account root is unreadable".to_string());
                continue;
            }
        };
        if !seen_roots.insert(canonical_root.clone()) {
            if is_pinned && matches!(root.try_exists(), Ok(false)) {
                issues.push("Codex pinned account root is unavailable".to_string());
            }
            continue;
        }
        match account_from_root(&canonical_root, home, is_default) {
            Ok(Some((account, stable_identity))) => {
                if stable_identity
                    .as_ref()
                    .is_some_and(|identity| !seen_identities.insert(identity.clone()))
                {
                    continue;
                }
                if !seen_keys.insert(account.key.clone()) {
                    issues.push("Codex account key collision".to_string());
                    continue;
                }
                accounts.push(account);
            }
            Ok(None) if is_pinned && matches!(root.try_exists(), Ok(false)) => {
                issues.push("Codex pinned account root is unavailable".to_string());
            }
            Ok(None) => {}
            Err(issue) => issues.push(issue),
        }
    }
    AccountDiscovery { accounts, issues }
}

/// Compatibility helper for callers that already resolved one root.
pub fn discover_accounts(codex_home: &Path) -> Vec<Account> {
    let root = fs::canonicalize(codex_home).unwrap_or_else(|_| codex_home.to_path_buf());
    let home = root.parent().unwrap_or(&root);
    account_from_root(&root, home, true)
        .ok()
        .flatten()
        .map(|(account, _)| vec![account])
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TokenCounts {
    input: u64,
    output: u64,
    cached: u64,
    reasoning: u64,
}

impl TokenCounts {
    fn from_value(obj: &Value) -> Self {
        let n = |key: &str| obj.get(key).and_then(Value::as_u64).unwrap_or(0);
        Self {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cached: n("cached_input_tokens"),
            reasoning: n("reasoning_output_tokens"),
        }
    }

    fn delta_from(self, previous: Self) -> Self {
        Self {
            input: self.input.saturating_sub(previous.input),
            output: self.output.saturating_sub(previous.output),
            cached: self.cached.saturating_sub(previous.cached),
            reasoning: self.reasoning.saturating_sub(previous.reasoning),
        }
    }

    fn normalized(self) -> (u64, u64, u64, u64) {
        (
            self.input.saturating_sub(self.cached),
            self.output,
            self.cached,
            self.reasoning,
        )
    }

    fn is_empty(self) -> bool {
        self.input == 0 && self.output == 0 && self.cached == 0 && self.reasoning == 0
    }
}

#[derive(Clone, Copy, Debug)]
struct CumulativeUsage {
    counts: TokenCounts,
    total: u64,
}

impl CumulativeUsage {
    fn from_value(obj: &Value) -> Option<Self> {
        let counts = TokenCounts::from_value(obj);
        let total = obj
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| counts.input.saturating_add(counts.output));
        (total > 0 || !counts.is_empty()).then_some(Self { counts, total })
    }
}

fn token_count_info(v: &Value) -> Option<&Value> {
    if v.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = v.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    Some(payload.get("info").unwrap_or(payload))
}

/// `window_minutes` of the rolling 5-hour window.
const FIVE_HOUR_WINDOW_MINUTES: u64 = 300;
/// `window_minutes` of the rolling weekly window.
const WEEK_WINDOW_MINUTES: u64 = 10_080;
/// The `limit_id` of the account-wide Codex limit. Other ids name separate
/// limits and must not stand in for the account's position.
const ACCOUNT_LIMIT_ID: &str = "codex";

/// One rate-limit window as Codex recorded it (`primary` / `secondary`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateLimitWindow {
    pub used_percent: f64,
    pub window_minutes: u64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// The rate-limit position Codex recorded with one `token_count` event: the
/// provider's own figures, not an estimate.
#[derive(Clone, Debug, PartialEq)]
pub struct RateLimitSnapshot {
    /// Timestamp of the rollout line that carried it.
    pub observed_at: DateTime<Utc>,
    pub windows: Vec<RateLimitWindow>,
}

impl RateLimitSnapshot {
    fn window(&self, window_minutes: u64) -> Option<&RateLimitWindow> {
        self.windows
            .iter()
            .find(|window| window.window_minutes == window_minutes)
    }
}

fn rate_limit_window(value: &Value) -> Option<RateLimitWindow> {
    let used_percent = value.get("used_percent").and_then(Value::as_f64)?;
    let window_minutes = value.get("window_minutes").and_then(Value::as_u64)?;
    let resets_at = value
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
    used_percent.is_finite().then_some(RateLimitWindow {
        used_percent,
        window_minutes,
        resets_at,
    })
}

/// The account-wide `rate_limits` of a `token_count` payload, if it carries one.
fn rate_limit_snapshot(payload: &Value, observed_at: DateTime<Utc>) -> Option<RateLimitSnapshot> {
    let limits = payload
        .get("rate_limits")
        .filter(|limits| limits.is_object())?;
    if limits
        .get("limit_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != ACCOUNT_LIMIT_ID)
    {
        return None;
    }
    let windows = ["primary", "secondary"]
        .into_iter()
        .filter_map(|slot| limits.get(slot).and_then(rate_limit_window))
        .collect();
    Some(RateLimitSnapshot {
        observed_at,
        windows,
    })
}

/// Set an account's 5h and weekly heat from Codex's own rate-limit report.
///
/// A window is known only when the latest report carries it and its reset lies
/// after `now`; its heat is then `used_percent / 100`, real, with the reported
/// reset. Every other window is [`UsageProvenance::Unknown`] — Codex usage is
/// never estimated against a token budget, whose scale has nothing to do with
/// Codex's quotas.
pub fn apply_rate_limits(
    usage: &mut AccountUsage,
    snapshot: Option<&RateLimitSnapshot>,
    now: DateTime<Utc>,
) {
    let known = |window_minutes: u64| {
        snapshot
            .and_then(|snapshot| snapshot.window(window_minutes))
            .and_then(|window| {
                let resets_at = window.resets_at.filter(|resets_at| *resets_at > now)?;
                Some((window.used_percent / 100.0, resets_at))
            })
    };
    match known(FIVE_HOUR_WINDOW_MINUTES) {
        Some((heat, resets_at)) => {
            usage.heat = heat;
            usage.reset5h = Some(resets_at);
            usage.provenance_5h = UsageProvenance::Real;
        }
        None => {
            usage.heat = 0.0;
            usage.reset5h = None;
            usage.provenance_5h = UsageProvenance::Unknown;
        }
    }
    match known(WEEK_WINDOW_MINUTES) {
        Some((heat, resets_at)) => {
            usage.heat_week = heat;
            usage.reset_week = Some(resets_at);
            usage.provenance_week = UsageProvenance::Real;
        }
        None => {
            usage.heat_week = 0.0;
            usage.reset_week = None;
            usage.provenance_week = UsageProvenance::Unknown;
        }
    }
    // Codex reports no per-model sublimits.
    usage.heat_opus = None;
    usage.heat_sonnet = None;
}

/// The more recent of two rate-limit reports; a later line wins a tie.
fn latest_rate_limits(
    current: Option<RateLimitSnapshot>,
    candidate: Option<RateLimitSnapshot>,
) -> Option<RateLimitSnapshot> {
    match (current, candidate) {
        (Some(current), Some(candidate)) if candidate.observed_at < current.observed_at => {
            Some(current)
        }
        (current, None) => current,
        (_, candidate) => candidate,
    }
}

fn usage_entry(
    counts: TokenCounts,
    ts: DateTime<Utc>,
    model: &str,
    session_id: &str,
) -> Option<UsageEntry> {
    let (input, output, cached, reasoning) = counts.normalized();
    [input, output, cached, reasoning]
        .into_iter()
        .any(|count| count > 0)
        .then(|| UsageEntry {
            ts,
            provider: Provider::Codex,
            model: model.to_string(),
            input,
            output,
            cache_create: 0,
            cache_read: cached,
            reasoning,
            session_id: session_id.to_string(),
        })
}

fn confirmed_usage(
    info: &Value,
    previous_total: &mut Option<CumulativeUsage>,
) -> Option<TokenCounts> {
    let last = info
        .get("last_token_usage")
        .map(TokenCounts::from_value)
        .filter(|counts| !counts.is_empty());
    let Some(current) = info
        .get("total_token_usage")
        .and_then(CumulativeUsage::from_value)
    else {
        // Older token_count records did not include cumulative totals. They cannot
        // be deduplicated without also dropping equal-sized real turns.
        return last;
    };

    let selected = match *previous_total {
        None => last.or(Some(current.counts)),
        Some(previous) if current.total > previous.total => {
            last.or_else(|| Some(current.counts.delta_from(previous.counts)))
        }
        Some(previous) if current.total == previous.total => None,
        Some(previous) => {
            debug_assert!(current.total < previous.total);
            None
        }
    };
    *previous_total = Some(current);
    selected
}

/// Parse a Codex `rollout-*.jsonl` session into per-turn usage entries.
///
/// Emits per-turn usage only for `token_count` events whose cumulative snapshot
/// advances. `file_date` (from the `YYYY/MM/DD` path) is the timestamp fallback
/// when a line carries none.
pub fn parse_transcript(path: &Path, file_date: DateTime<Utc>) -> Vec<UsageEntry> {
    read_transcript_usage(path, file_date)
        .map(|usage| usage.entries)
        .unwrap_or_default()
}

/// What one rollout contributes to its account's usage.
#[derive(Clone, Debug, Default, PartialEq)]
struct RolloutUsage {
    entries: Vec<UsageEntry>,
    /// The rollout's most recent account-wide rate-limit report.
    rate_limits: Option<RateLimitSnapshot>,
}

fn read_transcript_usage(path: &Path, file_date: DateTime<Utc>) -> std::io::Result<RolloutUsage> {
    let content = fs::read_to_string(path)?;
    let mut current_model = String::from("unknown");
    let mut current_ts = file_date;
    // Same rule discovery uses, from the same function: the file name names the
    // session until a `session_meta` line does it properly. Deriving it any other
    // way here would stamp spend with an id no session row carries.
    let mut session_id = crate::codex_sessions::session_id_from_path(path);
    let mut entries = Vec::new();
    let mut rate_limits = None;
    let mut previous_total = None;

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(id) = find(&v, "id").and_then(|x| x.as_str()) {
                session_id = id.to_string();
            }
        }
        if let Some(m) = find(&v, "model").and_then(|x| x.as_str()) {
            current_model = m.to_string();
        }
        if let Some(ts) = find(&v, "timestamp")
            .and_then(|x| x.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            current_ts = ts.with_timezone(&Utc);
        }
        let Some(info) = token_count_info(&v) else {
            continue;
        };
        if let Some(payload) = v.get("payload") {
            rate_limits = latest_rate_limits(rate_limits, rate_limit_snapshot(payload, current_ts));
        }
        if let Some(counts) = confirmed_usage(info, &mut previous_total) {
            if let Some(entry) = usage_entry(counts, current_ts, &current_model, &session_id) {
                entries.push(entry);
            }
        }
    }
    Ok(RolloutUsage {
        entries,
        rate_limits,
    })
}

/// Derive a coarse timestamp (midday UTC) from a `sessions/YYYY/MM/DD/` path, used as
/// the fallback when a rollout line carries no timestamp.
fn date_from_path(path: &Path) -> Option<DateTime<Utc>> {
    let comps: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let pos = comps.iter().position(|c| *c == "sessions")?;
    let y: i32 = comps.get(pos + 1)?.parse().ok()?;
    let m: u32 = comps.get(pos + 2)?.parse().ok()?;
    let d: u32 = comps.get(pos + 3)?.parse().ok()?;
    chrono::NaiveDate::from_ymd_opt(y, m, d)?
        .and_hms_opt(12, 0, 0)
        .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc))
}

/// Holds a week of heavy use, so follow-up usage scans stay incremental.
const USAGE_CACHE_LIMIT: usize = 2048;

#[derive(Clone, Debug)]
struct CachedCodexUsage {
    fingerprint: FileFingerprint,
    /// The timestamp fallback the entries were parsed with. A rollout outside
    /// the dated layout falls back to the scan cutoff, which moves every scan.
    file_date: DateTime<Utc>,
    usage: RolloutUsage,
    last_used: u64,
}

/// Bounded process-local cache of distilled Codex usage records, keyed by
/// rollout path. Raw rollout content is never retained, and an unchanged
/// rollout is neither read nor parsed again.
#[derive(Clone, Debug, Default)]
pub(crate) struct CodexUsageCache {
    entries: HashMap<PathBuf, CachedCodexUsage>,
    clock: u64,
    #[cfg(test)]
    parse_count: u64,
}

impl CodexUsageCache {
    fn parse_file(
        &mut self,
        path: &Path,
        file_date: DateTime<Utc>,
    ) -> std::io::Result<RolloutUsage> {
        let fingerprint = match file_fingerprint(path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.entries.remove(path);
                return Err(error);
            }
        };
        self.clock = self.clock.wrapping_add(1);
        if let Some(cached) = self.entries.get_mut(path) {
            if cached.fingerprint == fingerprint && cached.file_date == file_date {
                cached.last_used = self.clock;
                return Ok(cached.usage.clone());
            }
        }
        let usage = match read_transcript_usage(path, file_date) {
            Ok(usage) => usage,
            Err(error) => {
                self.entries.remove(path);
                return Err(error);
            }
        };
        #[cfg(test)]
        {
            self.parse_count = self.parse_count.saturating_add(1);
        }
        self.entries.insert(
            path.to_path_buf(),
            CachedCodexUsage {
                fingerprint,
                file_date,
                usage: usage.clone(),
                last_used: self.clock,
            },
        );
        Ok(usage)
    }

    /// Forgets rollouts under `sessions` that a complete walk no longer saw,
    /// then bounds the cache. Bounding during the walk would evict rollouts the
    /// same walk still needs.
    fn finish_walk(&mut self, sessions: &Path, seen: &HashSet<PathBuf>, walk_complete: bool) {
        if walk_complete {
            self.entries
                .retain(|path, _| !path.starts_with(sessions) || seen.contains(path));
        }
        while self.entries.len() > USAGE_CACHE_LIMIT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }
}

/// All Codex usage entries newer than `since`, from `<config_dir>/sessions/**/rollout-*.jsonl`.
///
/// The returned bool is `true` when the walk hit an I/O error other than a missing
/// `sessions/` dir — see [`crate::claude::usage_for_account`] for why the caller then
/// marks the snapshot degraded.
pub fn usage_for_account(account: &Account, since: DateTime<Utc>) -> (Vec<UsageEntry>, bool) {
    let usage = usage_for_account_with_cache(account, since, &mut CodexUsageCache::default());
    (usage.entries, usage.io_error)
}

/// One account's rollout scan: spend entries plus its latest rate-limit report.
#[derive(Clone, Debug, Default)]
pub(crate) struct CodexAccountUsage {
    pub(crate) entries: Vec<UsageEntry>,
    /// The most recent report across the account's rollouts touched since the
    /// cutoff. An older one has passed even its weekly reset.
    pub(crate) rate_limits: Option<RateLimitSnapshot>,
    /// See [`usage_for_account`].
    pub(crate) io_error: bool,
}

pub(crate) fn usage_for_account_with_cache(
    account: &Account,
    since: DateTime<Utc>,
    cache: &mut CodexUsageCache,
) -> CodexAccountUsage {
    let sessions = account.config_dir.join("sessions");
    let mut entries = Vec::new();
    let mut rate_limits = None;
    let mut io_error = false;
    let mut seen = HashSet::new();
    for result in WalkDir::new(&sessions) {
        let file = match result {
            Ok(f) => f,
            Err(e) => {
                // Only a missing `sessions/` root (depth 0, NotFound) is exempt; a
                // NotFound below it or any other error truncates the walk. See
                // `claude::usage_for_account`.
                let missing_root = e.depth() == 0
                    && e.io_error().map(std::io::Error::kind) == Some(std::io::ErrorKind::NotFound);
                if !missing_root {
                    io_error = true;
                }
                continue;
            }
        };
        if !file.file_type().is_file() {
            continue;
        }
        let name = file.file_name().to_str().unwrap_or("");
        if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
            continue;
        }
        let modified = match fs::metadata(file.path()).and_then(|meta| meta.modified()) {
            Ok(modified) => DateTime::<Utc>::from(modified),
            Err(_) => {
                io_error = true;
                continue;
            }
        };
        if modified < since {
            continue;
        }
        let file_date = date_from_path(file.path()).unwrap_or(since);
        seen.insert(file.path().to_path_buf());
        match cache.parse_file(file.path(), file_date) {
            Ok(parsed) => {
                entries.extend(parsed.entries.into_iter().filter(|entry| entry.ts >= since));
                rate_limits = latest_rate_limits(rate_limits, parsed.rate_limits);
            }
            Err(_) => io_error = true,
        }
    }
    cache.finish_walk(&sessions, &seen, !io_error);
    CodexAccountUsage {
        entries,
        rate_limits,
        io_error,
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
