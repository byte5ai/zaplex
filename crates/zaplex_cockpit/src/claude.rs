//! Claude Code account discovery + transcript usage parsing.
//!
//! Mirrors `claudeplex` `discover.ts`/`collect.ts`. Reads only account metadata
//! (`oauthAccount`) and per-message token counts — never tokens or message content.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::types::{Account, Provider, UsageEntry};

/// Directory-name fragments that mark a `.claude*` dir as a backup/scratch copy, not
/// a real account (mirrors `claudeplex` discover.ts exclusions).
const EXCLUDE_FRAGMENTS: &[&str] = &["mem", "backup", "bak", "old", "tmp", "temp", "observer"];

fn is_excluded(dir_name: &str) -> bool {
    EXCLUDE_FRAGMENTS.iter().any(|f| dir_name.contains(f))
}

/// Resolve the `.claude.json` identity file for a config dir: prefer `<dir>/.claude.json`,
/// and for the default `~/.claude` fall back to `~/.claude.json` (the CLI's home file).
fn identity_json(config_dir: &Path, home: &Path, is_default: bool) -> Option<PathBuf> {
    let inside = config_dir.join(".claude.json");
    if inside.is_file() {
        return Some(inside);
    }
    if is_default {
        let home_file = home.join(".claude.json");
        if home_file.is_file() {
            return Some(home_file);
        }
    }
    None
}

/// A config dir qualifies as an account if it has an identity file or a
/// `projects/`/`sessions/` subdir.
fn dir_qualifies(config_dir: &Path, home: &Path, is_default: bool) -> bool {
    identity_json(config_dir, home, is_default).is_some()
        || config_dir.join("projects").is_dir()
        || config_dir.join("sessions").is_dir()
}

/// Stable account key from the config dir, e.g. `claude:default`, `claude:work`.
fn account_key(config_dir: &Path, is_default: bool) -> String {
    if is_default {
        return "claude:default".to_string();
    }
    let name = config_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("account");
    // ".claude-work" → "work"; otherwise the raw dir name.
    let suffix = name
        .strip_prefix(".claude-")
        .or_else(|| name.strip_prefix(".claude"))
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    format!("claude:{suffix}")
}

/// Derive a plan label from `organizationRateLimitTier` / `organizationType`.
fn plan_label(rate_tier: Option<&str>, org_type: Option<&str>) -> Option<String> {
    if let Some(tier) = rate_tier {
        if let Some(rest) = tier.strip_prefix("max_") {
            // "max_20x" → "Max 20x"
            return Some(format!("Max {rest}"));
        }
    }
    if org_type == Some("claude_max") {
        return Some("Max".to_string());
    }
    org_type.map(|t| t.strip_prefix("claude_").unwrap_or(t).to_string())
}

/// Build an [`Account`] from a config dir + its identity JSON. Returns `None` if the
/// dir does not represent a real account (no `oauthAccount` and no transcripts).
fn account_from_dir(config_dir: &Path, home: &Path, is_default: bool) -> Option<Account> {
    if !dir_qualifies(config_dir, home, is_default) {
        return None;
    }
    let oauth = identity_json(config_dir, home, is_default)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("oauthAccount").cloned());

    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
    let (email, display, org, role, plan) = match &oauth {
        Some(o) => (
            s(o, "emailAddress"),
            s(o, "displayName"),
            s(o, "organizationName"),
            s(o, "organizationRole"),
            plan_label(
                o.get("organizationRateLimitTier").and_then(|x| x.as_str()),
                o.get("organizationType").and_then(|x| x.as_str()),
            ),
        ),
        None => (None, None, None, None, None),
    };

    let label = email
        .clone()
        .or_else(|| display.clone())
        .or_else(|| org.clone())
        .unwrap_or_else(|| account_key(config_dir, is_default));

    Some(Account {
        provider: Provider::Claude,
        key: account_key(config_dir, is_default),
        config_dir: config_dir.to_path_buf(),
        label,
        email,
        org,
        role,
        plan_tier: plan,
        is_default,
    })
}

/// Discover Claude accounts: the default `~/.claude`, any `~/.claude-*` config dirs,
/// and `$CLAUDE_CONFIG_DIR`. (A process scan for live non-default dirs — discover.ts —
/// is deferred; see the design doc.)
pub fn discover_accounts(home: &Path, config_dir_env: Option<&str>) -> Vec<Account> {
    let mut candidates: Vec<(PathBuf, bool)> = Vec::new();
    candidates.push((home.join(".claude"), true));

    if let Ok(read) = fs::read_dir(home) {
        for entry in read.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name == ".claude" || !name.starts_with(".claude") {
                continue;
            }
            // .claude-* dirs (skip the plain-file ~/.claude.json and excluded copies).
            if is_excluded(name) || !entry.path().is_dir() {
                continue;
            }
            candidates.push((entry.path(), false));
        }
    }

    if let Some(env_dir) = config_dir_env.filter(|d| !d.is_empty()) {
        let p = PathBuf::from(env_dir);
        let is_default = p == home.join(".claude");
        if !candidates.iter().any(|(c, _)| *c == p) {
            candidates.push((p, is_default));
        }
    }

    let mut accounts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (dir, is_default) in candidates {
        if !seen.insert(dir.clone()) {
            continue;
        }
        if let Some(acct) = account_from_dir(&dir, home, is_default) {
            accounts.push(acct);
        }
    }
    accounts
}

/// Extract a [`UsageEntry`] from one parsed transcript line, or `None` if the line
/// is not an assistant turn with usage. Reads counts + model + timestamp only.
fn parse_line(v: &Value) -> Option<UsageEntry> {
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let message = v.get("message");
    let usage = message
        .and_then(|m| m.get("usage"))
        .or_else(|| v.get("usage"))?;
    let model = message
        .and_then(|m| m.get("model"))
        .or_else(|| v.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();
    let ts_str = v.get("timestamp").and_then(|t| t.as_str())?;
    let ts = DateTime::parse_from_rfc3339(ts_str)
        .ok()?
        .with_timezone(&Utc);
    let n = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    Some(UsageEntry {
        ts,
        provider: Provider::Claude,
        model,
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_create: n("cache_creation_input_tokens"),
        cache_read: n("cache_read_input_tokens"),
        reasoning: 0,
    })
}

/// Parse a single Claude `.jsonl` transcript into usage entries (skips malformed lines).
pub fn parse_transcript(path: &Path) -> Vec<UsageEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| parse_line(&v))
        .collect()
}

/// All usage entries for an account with a transcript mtime at or after `since`
/// (widest window cutoff). Walks `<config_dir>/projects/**/*.jsonl`.
pub fn usage_for_account(account: &Account, since: DateTime<Utc>) -> Vec<UsageEntry> {
    let projects = account.config_dir.join("projects");
    let mut entries = Vec::new();
    for file in WalkDir::new(&projects)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jsonl"))
    {
        // Cheap mtime prefilter: skip transcripts untouched since the cutoff.
        if let Ok(meta) = file.metadata() {
            if let Ok(modified) = meta.modified() {
                let modified: DateTime<Utc> = modified.into();
                if modified < since {
                    continue;
                }
            }
        }
        entries.extend(
            parse_transcript(file.path())
                .into_iter()
                .filter(|e| e.ts >= since),
        );
    }
    entries
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
