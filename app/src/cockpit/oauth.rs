//! The network half of C3b: one authenticated `GET /api/oauth/usage` per Claude
//! account, cached 15 minutes, feeding `zaplex_cockpit::apply_oauth_usage`.
//!
//! Policy line (C3b design §4): the cockpit may ask "how full is my quota?",
//! it may **never spend** the quota. This module talks to exactly one endpoint
//! and never touches model/completions APIs.
//!
//! Token hygiene (hard rules): the OAuth access token is read, used for this
//! one request, and dropped — never logged, never persisted, never shown in UI
//! or errors. Failures collapse to `None` (the caller keeps the estimate);
//! there are no error payloads to leak into.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::lock::Mutex;
use instant::Instant;
use zaplex_cockpit::{OauthUsage, UtilizationScale};

const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
/// The endpoint has an aggressive per-token 429 budget (~5 requests), so we
/// fetch at most one request per account per TTL — same cadence as the
/// claudeplex-desktop reference.
const TTL: Duration = Duration::from_secs(15 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn build_oauth_client_with_proxy(
    proxy_config: http_client::ProxyConfig,
) -> Result<reqwest::Client, reqwest::Error> {
    proxy_config
        .apply(reqwest::Client::builder())
        .timeout(REQUEST_TIMEOUT)
        .build()
}

/// One cached per-account result. `usage: None` records a failed attempt so we
/// do not hammer the endpoint (or the keychain) again before the TTL elapses.
#[derive(Clone, Copy, Debug)]
pub struct CachedOauth {
    pub usage: Option<OauthUsage>,
    pub fetched_at: Instant,
}

/// Shared per-model OAuth cache. Clones refer to the same entries, so even if
/// callers accidentally overlap, they cannot start requests from independent
/// stale snapshots of the cache.
#[derive(Clone, Default)]
pub struct OauthCache {
    entries: Arc<Mutex<HashMap<PathBuf, CachedOauth>>>,
    refresh_gate: Arc<Mutex<()>>,
}

impl OauthCache {
    async fn snapshot(&self) -> HashMap<PathBuf, CachedOauth> {
        self.entries.lock().await.clone()
    }
}

/// Read the OAuth access token for one account: `<config_dir>/.credentials.json`
/// (`.claudeAiOauth.accessToken`), with a macOS-keychain fallback for the
/// **default** login only — the keychain entry "Claude Code-credentials" holds
/// one token (the OS-default login's); reusing it for other accounts would
/// report one account's quota for all of them.
async fn read_access_token(config_dir: PathBuf, default_config_dir: PathBuf) -> Option<String> {
    let credentials_path = config_dir.join(".credentials.json");
    if let Some(token) = tokio::task::spawn_blocking(move || {
        let raw = std::fs::read_to_string(credentials_path).ok()?;
        parse_access_token(&raw)
    })
    .await
    .ok()
    .flatten()
    {
        return Some(token);
    }
    #[cfg(target_os = "macos")]
    if config_dir == default_config_dir {
        let mut command = command::r#async::Command::new("security");
        command
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .kill_on_drop(true);
        let output = tokio::time::timeout(KEYCHAIN_TIMEOUT, command.output())
            .await
            .ok()?
            .ok()?;
        if output.status.success() {
            return parse_access_token(std::str::from_utf8(&output.stdout).ok()?);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = default_config_dir;
    None
}

fn parse_access_token(raw: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    let token = json
        .get("claudeAiOauth")
        .and_then(|oauth| oauth.get("accessToken"))
        .and_then(|token| token.as_str())?;
    (!token.is_empty()).then(|| token.to_string())
}

/// GET the usage endpoint with the account's bearer token. Any failure —
/// missing credentials, 401/403 (stale token), 429 (rate limited), network
/// down, schema drift — returns `None`: fall back to the estimate.
async fn fetch_one(client: &reqwest::Client, token: &str) -> Option<OauthUsage> {
    let response = client
        .get(ENDPOINT)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().await.ok()?;
    parse_response(&body)
}

fn parse_response(body: &str) -> Option<OauthUsage> {
    // The versioned oauth-2025-04-20 endpoint contract reports
    // `utilization` in percent. Choose the scale once at this boundary so a
    // low value is never reinterpreted independently from the other windows.
    zaplex_cockpit::parse_oauth_usage(body, UtilizationScale::Percent)
}

/// Refresh every account whose cache entry is missing or older than the TTL,
/// returning the updated cache. Runs on the background executor; the futures
/// need a tokio reactor, so the caller wraps this in `async_compat`.
///
/// Fresh entries are passed through untouched — at most one request per
/// account per TTL, matching the endpoint's tight 429 budget.
pub async fn refresh_cache(
    claude_config_dirs: Vec<PathBuf>,
    default_config_dir: PathBuf,
    cache: OauthCache,
) -> HashMap<PathBuf, CachedOauth> {
    let Ok(client) = build_oauth_client_with_proxy(http_client::current_proxy_config()) else {
        return cache.snapshot().await;
    };
    refresh_cache_with(claude_config_dirs, cache, move |dir| {
        let client = client.clone();
        let default_config_dir = default_config_dir.clone();
        async move {
            match read_access_token(dir.clone(), default_config_dir).await {
                Some(token) => fetch_one(&client, &token).await,
                None => None,
            }
        }
    })
    .await
}

async fn refresh_cache_with<F, Fut>(
    mut claude_config_dirs: Vec<PathBuf>,
    cache: OauthCache,
    fetch: F,
) -> HashMap<PathBuf, CachedOauth>
where
    F: Fn(PathBuf) -> Fut,
    Fut: Future<Output = Option<OauthUsage>>,
{
    claude_config_dirs.sort();
    claude_config_dirs.dedup();

    // Serialize refresh flights without holding the cache mutex across file, keychain, or network
    // waits. A concurrent caller waits here, then observes the first flight's fresh entries.
    let _refresh_flight = cache.refresh_gate.lock().await;
    let stale: Vec<PathBuf> = {
        let mut entries = cache.entries.lock().await;
        entries.retain(|dir, _| claude_config_dirs.contains(dir));
        claude_config_dirs
            .into_iter()
            .filter(|dir| {
                entries
                    .get(dir)
                    .is_none_or(|cached| cached.fetched_at.elapsed() >= TTL)
            })
            .collect()
    };
    if stale.is_empty() {
        return cache.snapshot().await;
    }

    let fetches = stale.into_iter().map(|dir| {
        let future = fetch(dir.clone());
        async move { (dir, future.await) }
    });
    let fetched = futures::future::join_all(fetches).await;
    let mut entries = cache.entries.lock().await;
    for (dir, usage) in fetched {
        entries.insert(
            dir,
            CachedOauth {
                usage,
                fetched_at: Instant::now(),
            },
        );
    }
    entries.clone()
}

/// The merge view of a cache: only successful results, keyed by config dir.
pub fn usable_usage(cache: &HashMap<PathBuf, CachedOauth>) -> HashMap<PathBuf, OauthUsage> {
    cache
        .iter()
        .filter_map(|(dir, c)| c.usage.map(|u| (dir.clone(), u)))
        .collect()
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
