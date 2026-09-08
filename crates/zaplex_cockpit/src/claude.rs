//! Claude Code account discovery + transcript usage parsing.
//!
//! Mirrors `claudeplex` `discover.ts`/`collect.ts`. Reads only account metadata
//! (`oauthAccount`) and per-message token counts — never tokens or message content.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;

use chrono::{DateTime, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::types::{Account, Provider, UsageEntry};

/// Directory-name fragments that mark a `.claude*` dir as a backup/scratch copy, not
/// a real account (mirrors `claudeplex` discover.ts exclusions).
const EXCLUDE_FRAGMENTS: &[&str] = &["mem", "backup", "bak", "old", "tmp", "temp", "observer"];

/// Result of account-root discovery before transcript/session scanning.
#[derive(Debug, Default)]
pub struct AccountDiscovery {
    pub accounts: Vec<Account>,
    pub issues: Vec<String>,
}

#[derive(Debug, Default)]
struct ProcessAccountDiscovery {
    roots: Vec<PathBuf>,
    issues: Vec<String>,
}

#[cfg(target_os = "linux")]
const PROCESS_DISCOVERY_PERMISSION_DENIED: &str =
    "Claude process account discovery was denied by the operating system";
#[cfg(target_os = "linux")]
const PROCESS_DISCOVERY_INCOMPLETE: &str =
    "Claude process account discovery could not inspect every process";
#[cfg(target_os = "linux")]
const PROCESS_DISCOVERY_UNAVAILABLE: &str = "Claude process account discovery is unavailable";
fn push_unique_issue(issues: &mut Vec<String>, issue: impl Into<String>) {
    let issue = issue.into();
    if !issues.contains(&issue) {
        issues.push(issue);
    }
}

#[cfg(target_os = "linux")]
fn effective_uid(status: &[u8]) -> Option<u32> {
    let status = std::str::from_utf8(status).ok()?;
    let uid = status.lines().find_map(|line| line.strip_prefix("Uid:"))?;
    uid.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn process_arg_basename(argument: &[u8]) -> &[u8] {
    argument
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or(argument)
}

#[cfg(target_os = "linux")]
fn process_start_time(stat: &[u8]) -> Option<u64> {
    let command_end = stat.iter().rposition(|byte| *byte == b')')?;
    std::str::from_utf8(stat.get(command_end + 1..)?)
        .ok()?
        .split_whitespace()
        // `/proc/<pid>/stat` field 3 starts after the command; starttime is 22.
        .nth(19)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
fn is_claude_process(cmdline: &[u8]) -> bool {
    let mut arguments = cmdline
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty());
    let Some(executable) = arguments.next() else {
        return false;
    };
    let executable = process_arg_basename(executable);
    if executable == b"claude" || executable == b"claude-code" {
        return true;
    }
    if executable != b"node"
        && executable != b"nodejs"
        && executable != b"bun"
        && executable != b"deno"
    {
        return false;
    }
    let Some(script) = arguments.next() else {
        return false;
    };
    process_arg_basename(script) == b"claude"
        || process_arg_basename(script) == b"claude-code"
        || script
            .split(|byte| *byte == b'/')
            .any(|component| component == b"claude-code")
}

#[cfg(target_os = "linux")]
fn config_dir_from_environment(environ: &[u8]) -> Option<PathBuf> {
    const PREFIX: &[u8] = b"CLAUDE_CONFIG_DIR=";
    let value = environ
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(PREFIX))?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(value.to_vec())))
}

#[cfg(target_os = "linux")]
fn read_process_file(
    path: &Path,
    read: &impl Fn(&Path) -> io::Result<Vec<u8>>,
    issues: &mut Vec<String>,
) -> Option<Vec<u8>> {
    match read(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            push_unique_issue(issues, PROCESS_DISCOVERY_PERMISSION_DENIED);
            None
        }
        Err(_) => {
            push_unique_issue(issues, PROCESS_DISCOVERY_INCOMPLETE);
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn read_process_bytes(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

#[cfg(target_os = "linux")]
fn running_claude_config_dirs_from_proc_with_reader(
    proc_root: &Path,
    read: &impl Fn(&Path) -> io::Result<Vec<u8>>,
) -> ProcessAccountDiscovery {
    let mut discovery = ProcessAccountDiscovery::default();
    let own_status = match read(&proc_root.join("self/status")) {
        Ok(status) => status,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            discovery
                .issues
                .push(PROCESS_DISCOVERY_PERMISSION_DENIED.to_string());
            return discovery;
        }
        Err(_) => {
            discovery
                .issues
                .push(PROCESS_DISCOVERY_UNAVAILABLE.to_string());
            return discovery;
        }
    };
    let Some(own_uid) = effective_uid(&own_status) else {
        discovery
            .issues
            .push(PROCESS_DISCOVERY_UNAVAILABLE.to_string());
        return discovery;
    };

    let entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            discovery
                .issues
                .push(PROCESS_DISCOVERY_PERMISSION_DENIED.to_string());
            return discovery;
        }
        Err(_) => {
            discovery
                .issues
                .push(PROCESS_DISCOVERY_UNAVAILABLE.to_string());
            return discovery;
        }
    };

    let mut process_roots = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_PERMISSION_DENIED);
                continue;
            }
            Err(_) => {
                push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_INCOMPLETE);
                continue;
            }
        };
        let Some(pid) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }

        let process_root = entry.path();
        let Some(status) =
            read_process_file(&process_root.join("status"), read, &mut discovery.issues)
        else {
            continue;
        };
        let Some(uid) = effective_uid(&status) else {
            push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_INCOMPLETE);
            continue;
        };
        if uid != own_uid {
            continue;
        }

        let Some(stat) = read_process_file(&process_root.join("stat"), read, &mut discovery.issues)
        else {
            continue;
        };
        let Some(start_time) = process_start_time(&stat) else {
            push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_INCOMPLETE);
            continue;
        };

        let Some(cmdline) =
            read_process_file(&process_root.join("cmdline"), read, &mut discovery.issues)
        else {
            continue;
        };
        if !is_claude_process(&cmdline) {
            continue;
        }

        let Some(environ) =
            read_process_file(&process_root.join("environ"), read, &mut discovery.issues)
        else {
            continue;
        };
        let Some(rechecked_status) =
            read_process_file(&process_root.join("status"), read, &mut discovery.issues)
        else {
            continue;
        };
        let Some(rechecked_stat) =
            read_process_file(&process_root.join("stat"), read, &mut discovery.issues)
        else {
            continue;
        };
        let Some(rechecked_cmdline) =
            read_process_file(&process_root.join("cmdline"), read, &mut discovery.issues)
        else {
            continue;
        };
        if effective_uid(&rechecked_status) != Some(own_uid)
            || process_start_time(&rechecked_stat) != Some(start_time)
            || rechecked_cmdline != cmdline
        {
            continue;
        }
        let Some(mut config_dir) = config_dir_from_environment(&environ) else {
            continue;
        };
        if config_dir.is_relative() {
            config_dir = match fs::read_link(process_root.join("cwd")) {
                Ok(cwd) => cwd.join(config_dir),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_PERMISSION_DENIED);
                    continue;
                }
                Err(_) => {
                    push_unique_issue(&mut discovery.issues, PROCESS_DISCOVERY_INCOMPLETE);
                    continue;
                }
            };
        }
        process_roots.push(config_dir);
    }
    process_roots.sort();
    process_roots.dedup();
    discovery.roots = process_roots;
    discovery
}

#[cfg(target_os = "linux")]
fn running_claude_config_dirs() -> ProcessAccountDiscovery {
    running_claude_config_dirs_from_proc_with_reader(Path::new("/proc"), &read_process_bytes)
}

#[cfg(not(target_os = "linux"))]
fn running_claude_config_dirs() -> ProcessAccountDiscovery {
    // Process inspection is an optional discovery source. A platform without
    // it still has an authoritative scan of static and explicitly pinned roots.
    ProcessAccountDiscovery::default()
}

fn is_excluded(dir_name: &str) -> bool {
    dir_name
        .strip_prefix(".claude-")
        .unwrap_or(dir_name)
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|token| EXCLUDE_FRAGMENTS.contains(&token) || token == "memory")
}

/// Resolve the `.claude.json` identity file for a config dir: prefer `<dir>/.claude.json`,
/// and for the default `~/.claude` fall back to `~/.claude.json` (the CLI's home file).
fn identity_json(
    config_dir: &Path,
    home: &Path,
    is_default: bool,
) -> Result<Option<PathBuf>, String> {
    let inside = config_dir.join(".claude.json");
    match inside.try_exists() {
        Ok(true) => return Ok(Some(inside)),
        Ok(false) => {}
        Err(_) => return Err("Claude account identity is unreadable".to_string()),
    }
    if is_default {
        let home_file = home.join(".claude.json");
        match home_file.try_exists() {
            Ok(true) => return Ok(Some(home_file)),
            Ok(false) => {}
            Err(_) => return Err("Claude default account identity is unreadable".to_string()),
        }
    }
    Ok(None)
}

fn store_exists(config_dir: &Path, name: &str) -> Result<bool, String> {
    match fs::metadata(config_dir.join(name)) {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("Claude account store is unreadable".to_string()),
    }
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
/// dir has neither an identity file nor a provider-owned session store.
fn account_from_dir(
    config_dir: &Path,
    home: &Path,
    is_default: bool,
) -> Result<Option<(Account, Option<String>)>, String> {
    let identity_path = identity_json(config_dir, home, is_default)?;
    let has_store = store_exists(config_dir, "projects")? || store_exists(config_dir, "sessions")?;
    if identity_path.is_none() && !has_store {
        return Ok(None);
    }

    let oauth = match identity_path {
        Some(path) => {
            let raw = fs::read_to_string(path)
                .map_err(|_| "Claude account identity is unreadable".to_string())?;
            let identity = serde_json::from_str::<Value>(&raw)
                .map_err(|_| "Claude account identity is malformed".to_string())?;
            match identity.get("oauthAccount") {
                Some(Value::Object(_)) => identity.get("oauthAccount").cloned(),
                Some(_) => return Err("Claude account identity is malformed".to_string()),
                None => None,
            }
        }
        None => None,
    };
    // The default data directory exists for every Claude installation, including
    // logged-out ones. Only its home-level OAuth identity makes it a real account;
    // otherwise a historical store would invent a noisy `claude:default` card.
    if is_default && oauth.is_none() {
        return Ok(None);
    }

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

    let stable_identity = oauth.as_ref().and_then(|o| {
        s(o, "accountUuid")
            .map(|id| id.to_ascii_lowercase())
            .or_else(|| {
                let email = o
                    .get("emailAddress")
                    .or_else(|| o.get("email"))
                    .and_then(Value::as_str)?;
                let organization = o.get("organizationUuid").and_then(Value::as_str)?;
                Some(format!(
                    "{}:{}",
                    email.to_ascii_lowercase(),
                    organization.to_ascii_lowercase()
                ))
            })
    });

    Ok(Some((
        Account {
            provider: Provider::Claude,
            key: account_key(config_dir, is_default),
            config_dir: config_dir.to_path_buf(),
            label,
            email,
            org,
            role,
            plan_tier: plan,
            is_default,
        },
        stable_identity,
    )))
}

/// Discover Claude accounts and retain root/identity failures for snapshot health.
///
/// Sources are the documented default `~/.claude`, sorted `~/.claude-*` siblings,
/// same-UID live Claude processes, and the root pinned by `$CLAUDE_CONFIG_DIR`.
/// Existing roots are canonicalized so aliases across sources do not create
/// duplicate accounts.
fn discover_accounts_with_process_roots(
    home: &Path,
    config_dir_env: Option<&str>,
    process_discovery: ProcessAccountDiscovery,
) -> AccountDiscovery {
    let mut candidates: Vec<(PathBuf, bool, bool)> = Vec::new();
    candidates.push((home.join(".claude"), true, false));

    let mut issues = Vec::new();
    match fs::read_dir(home) {
        Ok(read) => {
            let mut siblings = Vec::new();
            for entry in read {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        issues.push("Claude account directory entry is unreadable".to_string());
                        continue;
                    }
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name == ".claude" || !name.starts_with(".claude-") {
                    continue;
                }
                if is_excluded(name) {
                    continue;
                }
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() || kind.is_symlink() => siblings.push(entry.path()),
                    Ok(_) => {}
                    Err(_) => {
                        issues.push("Claude account root is unreadable".to_string());
                    }
                }
            }
            siblings.sort();
            candidates.extend(siblings.into_iter().map(|path| (path, false, false)));
        }
        Err(_) if !matches!(home.try_exists(), Ok(false)) => {
            issues.push("Claude accounts: home directory unreadable".to_string());
        }
        Err(_) => {}
    }

    for root in process_discovery.roots {
        let is_default = root == home.join(".claude");
        candidates.push((root, is_default, false));
    }
    for issue in process_discovery.issues {
        push_unique_issue(&mut issues, issue);
    }

    if let Some(env_dir) = config_dir_env.filter(|d| !d.is_empty()) {
        let p = PathBuf::from(env_dir);
        let is_default = p == home.join(".claude");
        candidates.push((p, is_default, true));
    }

    let mut accounts = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut seen_identities = HashSet::new();
    for (dir, is_default, is_pinned) in candidates {
        let root = match fs::canonicalize(&dir) {
            Ok(root) => root,
            Err(_) if fs::symlink_metadata(&dir).is_ok() => {
                issues.push("Claude account root is unreadable".to_string());
                continue;
            }
            Err(_) if matches!(dir.try_exists(), Ok(false)) => dir.clone(),
            Err(_) => {
                issues.push("Claude account root is unreadable".to_string());
                continue;
            }
        };
        if !seen_roots.insert(root.clone()) {
            if is_pinned && matches!(dir.try_exists(), Ok(false)) {
                issues.push("Claude pinned account root is unavailable".to_string());
            }
            continue;
        }
        match account_from_dir(&root, home, is_default) {
            Ok(Some((account, stable_identity))) => {
                if stable_identity
                    .as_ref()
                    .is_some_and(|identity| !seen_identities.insert(identity.clone()))
                {
                    continue;
                }
                accounts.push(account);
            }
            Ok(None) => {
                if is_pinned && matches!(dir.try_exists(), Ok(false)) {
                    issues.push("Claude pinned account root is unavailable".to_string());
                }
            }
            Err(issue) => issues.push(issue),
        }
    }
    AccountDiscovery { accounts, issues }
}

/// Discover Claude accounts and retain root/identity/process failures for health.
pub fn discover_accounts_with_health(
    home: &Path,
    config_dir_env: Option<&str>,
) -> AccountDiscovery {
    discover_accounts_with_process_roots(home, config_dir_env, running_claude_config_dirs())
}

/// Compatibility helper for callers that only need the discovered accounts.
pub fn discover_accounts(home: &Path, config_dir_env: Option<&str>) -> Vec<Account> {
    discover_accounts_with_health(home, config_dir_env).accounts
}

struct AssistantUsageSnapshot {
    entry: UsageEntry,
    request_id: Option<String>,
    message_id: Option<String>,
    stop_reason: Option<String>,
}

impl AssistantUsageSnapshot {
    fn request_key(&self) -> Option<(&str, &str)> {
        Some((self.request_id.as_deref()?, self.message_id.as_deref()?))
    }

    fn should_replace(&self, candidate: &Self) -> bool {
        match (self.stop_reason.is_some(), candidate.stop_reason.is_some()) {
            (false, true) => true,
            (true, false) => false,
            (true, true) => true,
            (false, false) => candidate.entry.output >= self.entry.output,
        }
    }
}

/// Extract an assistant usage snapshot from one parsed transcript line.
/// `session_id` is supplied by the caller — see [`parse_transcript`].
fn parse_line(v: &Value, session_id: &str) -> Option<AssistantUsageSnapshot> {
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
    let non_empty_string = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some(AssistantUsageSnapshot {
        entry: UsageEntry {
            ts,
            provider: Provider::Claude,
            model,
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_create: n("cache_creation_input_tokens"),
            cache_read: n("cache_read_input_tokens"),
            reasoning: 0,
            session_id: session_id.to_string(),
        },
        request_id: non_empty_string(v.get("requestId")),
        message_id: non_empty_string(message.and_then(|message| message.get("id"))),
        stop_reason: non_empty_string(message.and_then(|message| message.get("stop_reason"))),
    })
}

/// Parse a single Claude `.jsonl` transcript into usage entries (skips malformed lines).
///
/// Every entry is stamped with the session the transcript belongs to, taken from
/// the **file name**. The lines carry a `sessionId` of their own and it agrees,
/// but the stem is the id the rest of the crate keys on: `transcripts_by_id`
/// indexes by it, and a session is only discovered when a transcript named after
/// its registry id exists. Reading the id from the content could therefore
/// attribute spend to an id no session row has.
pub fn parse_transcript(path: &Path) -> Vec<UsageEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let mut snapshots = Vec::<AssistantUsageSnapshot>::new();
    let mut keyed_snapshots = HashMap::<(String, String), usize>::new();
    for snapshot in content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| parse_line(&value, session_id))
    {
        let Some((request_id, message_id)) = snapshot.request_key() else {
            snapshots.push(snapshot);
            continue;
        };
        let key = (request_id.to_string(), message_id.to_string());
        if let Some(index) = keyed_snapshots.get(&key).copied() {
            if snapshots[index].should_replace(&snapshot) {
                snapshots[index] = snapshot;
            }
        } else {
            keyed_snapshots.insert(key, snapshots.len());
            snapshots.push(snapshot);
        }
    }
    snapshots
        .into_iter()
        .map(|snapshot| snapshot.entry)
        .collect()
}

/// All usage entries for an account with a transcript mtime at or after `since`
/// (widest window cutoff). Walks `<config_dir>/projects/**/*.jsonl`.
///
/// The returned bool is `true` when the directory walk hit an I/O error *other than*
/// a missing `projects/` dir (a permission error on a subdir, etc.) — the usage totals
/// may then be incomplete. Callers mark the snapshot degraded so a silently-truncated
/// scan is not mistaken for "never used" (which would also look maximally free to the
/// launcher's freest-account routing).
pub fn usage_for_account(account: &Account, since: DateTime<Utc>) -> (Vec<UsageEntry>, bool) {
    let projects = account.config_dir.join("projects");
    let mut entries = Vec::new();
    let mut io_error = false;
    for result in WalkDir::new(&projects) {
        let file = match result {
            Ok(f) => f,
            Err(e) => {
                // A missing `projects/` ROOT (depth 0, NotFound) is the "account never
                // used" case and fine. A NotFound below the root, or any other error
                // (permission on a subdir, etc.), means the walk was truncated and the
                // numbers may be incomplete.
                let missing_root = e.depth() == 0
                    && e.io_error().map(std::io::Error::kind) == Some(std::io::ErrorKind::NotFound);
                if !missing_root {
                    io_error = true;
                }
                continue;
            }
        };
        if !file.file_type().is_file()
            || file.path().extension().and_then(|x| x.to_str()) != Some("jsonl")
        {
            continue;
        }
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
    (entries, io_error)
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
