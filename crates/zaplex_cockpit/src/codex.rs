//! Codex account discovery + session (rollout) usage parsing.
//!
//! Net-new (no `claudeplex` prior art); the exact on-disk schema is only partly
//! confirmed (design doc §10), so parsing is deliberately **defensive**: it searches
//! each JSONL line for a token-usage object rather than assuming a fixed path.
//!
//! Privacy: reads `auth.json` only for `auth_mode` and decodes the **unverified**
//! `id_token` JWT payload for an `email` claim. Token strings are never stored.

use std::fs;
use std::path::Path;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::types::{Account, Provider, UsageEntry};

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

/// Discover the Codex account from `<codex_home>/auth.json`. Codex multi-account is
/// unconfirmed (design §10), so Increment 1 treats it as a single account.
pub fn discover_accounts(codex_home: &Path) -> Vec<Account> {
    let auth_path = codex_home.join("auth.json");
    let Ok(raw) = fs::read_to_string(&auth_path) else {
        return Vec::new();
    };
    let Ok(auth) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };

    let auth_mode = auth
        .get("auth_mode")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    // Email from the id_token JWT payload (best-effort; token itself is never stored).
    let email = auth
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|x| x.as_str())
        .and_then(jwt_payload)
        .and_then(|claims| {
            claims
                .get("email")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        });

    let label = email
        .clone()
        .or_else(|| auth_mode.clone())
        .unwrap_or_else(|| "codex".to_string());

    vec![Account {
        provider: Provider::Codex,
        key: "codex:default".to_string(),
        config_dir: codex_home.to_path_buf(),
        label,
        email,
        org: None,
        role: None,
        plan_tier: auth_mode, // best-effort until plan claim is confirmed (§10)
        is_default: true,
    }]
}

/// Read `input_tokens` / `output_tokens` / `cached_input_tokens` /
/// `reasoning_output_tokens` from a token-usage object.
fn tokens_from(obj: &Value) -> (u64, u64, u64, u64) {
    let n = |k: &str| obj.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    (
        n("input_tokens"),
        n("output_tokens"),
        n("cached_input_tokens"),
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
    let mut entries = Vec::new();

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
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
            if input + output + cached + reasoning > 0 {
                entries.push(UsageEntry {
                    ts: current_ts,
                    provider: Provider::Codex,
                    model: current_model.clone(),
                    input,
                    output,
                    cache_create: 0, // Codex has no separate cache-write concept
                    cache_read: cached,
                    reasoning,
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
pub fn usage_for_account(account: &Account, since: DateTime<Utc>) -> Vec<UsageEntry> {
    let sessions = account.config_dir.join("sessions");
    let mut entries = Vec::new();
    for file in WalkDir::new(&sessions)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
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
    entries
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
