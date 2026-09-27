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
use sha2::{Digest, Sha256};
use zaplex_cockpit::{Account, OauthUsage, UtilizationScale};

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
/// do not hammer the endpoint again before the TTL elapses.
type AccountIdentity = (Option<String>, Option<String>, Option<String>);

fn account_identity(account: &Account) -> AccountIdentity {
    (
        account.provider_account_id.clone(),
        account.email.clone(),
        account.org.clone(),
    )
}

#[derive(Clone)]
pub struct CachedOauth {
    identity: AccountIdentity,
    credential_fingerprint: [u8; 32],
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
    claude_accounts: Vec<Account>,
    default_config_dir: PathBuf,
    cache: OauthCache,
) -> HashMap<PathBuf, CachedOauth> {
    let Ok(client) = build_oauth_client_with_proxy(http_client::current_proxy_config()) else {
        // Cached credentials cannot be revalidated on this failed refresh.
        return HashMap::new();
    };
    refresh_cache_with(
        claude_accounts,
        cache,
        move |dir| read_access_token(dir, default_config_dir.clone()),
        move |token| {
            let client = client.clone();
            async move { fetch_one(&client, &token).await }
        },
    )
    .await
}

async fn refresh_cache_with<R, ReadFuture, F, FetchFuture>(
    mut claude_accounts: Vec<Account>,
    cache: OauthCache,
    read_token: R,
    fetch: F,
) -> HashMap<PathBuf, CachedOauth>
where
    R: Fn(PathBuf) -> ReadFuture,
    ReadFuture: Future<Output = Option<String>>,
    F: Fn(String) -> FetchFuture,
    FetchFuture: Future<Output = Option<OauthUsage>>,
{
    claude_accounts.sort_by(|a, b| a.config_dir.cmp(&b.config_dir));
    claude_accounts.dedup_by(|a, b| a.config_dir == b.config_dir);

    // Read credentials only after acquiring the flight slot: a queued refresh
    // must not carry a token captured before the previous request completed.
    let _refresh_flight = cache.refresh_gate.lock().await;
    cache.entries.lock().await.retain(|dir, _| {
        claude_accounts
            .iter()
            .any(|account| &account.config_dir == dir)
    });
    let fetches = claude_accounts.into_iter().map(|account| {
        let cache = &cache;
        let read_token = &read_token;
        let fetch = &fetch;
        async move {
            let dir = account.config_dir;
            let identity = (account.provider_account_id, account.email, account.org);
            let Some(token) = read_token(dir.clone()).await else {
                cache.entries.lock().await.remove(&dir);
                return;
            };
            let fingerprint: [u8; 32] = Sha256::digest(token.as_bytes()).into();
            let fresh = cache.entries.lock().await.get(&dir).is_some_and(|cached| {
                cached.identity == identity
                    && cached.credential_fingerprint == fingerprint
                    && cached.fetched_at.elapsed() < TTL
            });
            if fresh {
                return;
            }
            // Evict the former account before waiting on the network. A failed
            // or obsolete response must never resurrect its usage.
            cache.entries.lock().await.remove(&dir);
            let usage = fetch(token).await;
            let current_fingerprint = read_token(dir.clone())
                .await
                .map(|token| <[u8; 32]>::from(Sha256::digest(token.as_bytes())));
            if current_fingerprint != Some(fingerprint) {
                return;
            }
            cache.entries.lock().await.insert(
                dir,
                CachedOauth {
                    identity,
                    credential_fingerprint: fingerprint,
                    usage,
                    fetched_at: Instant::now(),
                },
            );
        }
    });
    futures::future::join_all(fetches).await;
    cache.snapshot().await
}

/// Merge only results belonging to both the scanned and freshly revalidated
/// account identity. A login changed during the request is retried next scan.
pub fn usable_usage(
    cache: &HashMap<PathBuf, CachedOauth>,
    scanned: &[Account],
    current: &[Account],
) -> HashMap<PathBuf, OauthUsage> {
    scanned
        .iter()
        .filter_map(|account| {
            let cached = cache.get(&account.config_dir)?;
            let identity = account_identity(account);
            let current_matches = current.iter().any(|now| {
                now.config_dir == account.config_dir && account_identity(now) == identity
            });
            (current_matches && cached.identity == identity && cached.fetched_at.elapsed() < TTL)
                .then(|| {
                    cached
                        .usage
                        .map(|usage| (account.config_dir.clone(), usage))
                })
                .flatten()
        })
        .collect()
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
