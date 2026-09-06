//! Codex account discovery + session (rollout) usage parsing.
//!
//! Net-new (no `claudeplex` prior art); the exact on-disk schema is only partly
//! confirmed (design doc §10), so parsing is deliberately **defensive**: it searches
//! each JSONL line for a token-usage object rather than assuming a fixed path.
//!
//! Privacy: reads `auth.json` only for `auth_mode` and decodes the **unverified**
//! `id_token` JWT payload for an `email` claim. Token strings are never stored.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::types::{Account, Provider, UsageEntry};

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
/// for auth — only to read a display `email` claim. Returns `None` on any malformation.
fn jwt_payload(token: &str) -> Option<Value> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
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
    let account_id = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);

    if auth_mode.is_none() && account_id.is_none() && email.is_none() {
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
            email,
            org: None,
            role: None,
            // Provider ≠ plan (WS4 S2): `auth_mode` ("chatgpt" / "apikey") is *how* you
            // authenticate, not a subscription plan. Leaking it into `plan_tier` made
            // the sidebar render "Codex · chatgpt" (provider in the plan slot). Codex
            // exposes no plan claim yet, so the plan is honestly unknown (`None`).
            plan_tier: None,
            is_default,
        },
        account_id,
    )))
}

/// Discover Codex accounts from the documented default `~/.codex` root and an
/// optional root pinned by `$CODEX_HOME`.
///
/// Existing roots are canonicalized, then aliases are deduplicated by canonical
/// path or the stable `account_id` in `auth.json`. Codex itself still exposes one
/// account per root; this function merely unions the deterministic roots.
pub fn discover_account_roots(home: &Path, codex_home_env: Option<&Path>) -> AccountDiscovery {
    let default_root = home.join(".codex");
    let mut candidates: Vec<(PathBuf, bool, bool)> = vec![(default_root.clone(), true, false)];
    if let Some(root) = codex_home_env {
        candidates.push((root.to_path_buf(), root == default_root.as_path(), true));
    }

    let mut accounts = Vec::new();
    let mut issues = Vec::new();
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

/// Read `input_tokens` / `output_tokens` / `cached_input_tokens` /
/// `reasoning_output_tokens` from a token-usage object.
fn tokens_from(obj: &Value) -> (u64, u64, u64, u64) {
    let n = |k: &str| obj.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let input = n("input_tokens");
    let cached = n("cached_input_tokens");
    (
        // Codex reports cached input as part of `input_tokens`. Store only the
        // uncached remainder here; `cache_read` owns the cached part so totals,
        // work, and pricing each count every token once.
        input.saturating_sub(cached),
        n("output_tokens"),
        cached,
        n("reasoning_output_tokens"),
    )
}

/// Parse a Codex `rollout-*.jsonl` session into per-turn usage entries.
///
/// Sums **per-turn** deltas (`last_token_usage`) to avoid double-counting the
/// cumulative `total_token_usage` envelope. `file_date` (from the `YYYY/MM/DD` path)
/// is the timestamp fallback when a line carries none.
pub fn parse_transcript(path: &Path, file_date: DateTime<Utc>) -> Vec<UsageEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut current_model = String::from("unknown");
    let mut current_ts = file_date;
    // Same rule discovery uses, from the same function: the file name names the
    // session until a `session_meta` line does it properly. Deriving it any other
    // way here would stamp spend with an id no session row carries.
    let mut session_id = crate::codex_sessions::session_id_from_path(path);
    let mut entries = Vec::new();

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
        // Per-turn usage; ignore cumulative `total_token_usage` to avoid double counts.
        if let Some(usage) = find(&v, "last_token_usage") {
            let (input, output, cached, reasoning) = tokens_from(usage);
            if [input, output, cached, reasoning]
                .into_iter()
                .any(|n| n > 0)
            {
                entries.push(UsageEntry {
                    ts: current_ts,
                    provider: Provider::Codex,
                    model: current_model.clone(),
                    input,
                    output,
                    cache_create: 0, // Codex has no separate cache-write concept
                    cache_read: cached,
                    reasoning,
                    session_id: session_id.clone(),
                });
            }
        }
    }
    entries
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

/// All Codex usage entries newer than `since`, from `<config_dir>/sessions/**/rollout-*.jsonl`.
///
/// The returned bool is `true` when the walk hit an I/O error other than a missing
/// `sessions/` dir — see [`crate::claude::usage_for_account`] for why the caller then
/// marks the snapshot degraded.
pub fn usage_for_account(account: &Account, since: DateTime<Utc>) -> (Vec<UsageEntry>, bool) {
    let sessions = account.config_dir.join("sessions");
    let mut entries = Vec::new();
    let mut io_error = false;
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
        if let Ok(meta) = file.metadata() {
            if let Ok(modified) = meta.modified() {
                let modified: DateTime<Utc> = modified.into();
                if modified < since {
                    continue;
                }
            }
        }
        let file_date = date_from_path(file.path()).unwrap_or(since);
        entries.extend(
            parse_transcript(file.path(), file_date)
                .into_iter()
                .filter(|e| e.ts >= since),
        );
    }
    (entries, io_error)
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
