// openWarp(Channel::Oss) autoupdate uses GitHub Releases API, not Zaplex's official
// channel_versions / GCS. This module only handles "fetch latest release metadata" + "select asset by filename";
// actual download, save, and directory opening are handled by windows.rs / mac.rs.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result};
use lazy_static::lazy_static;
use serde::Deserialize;

const REPO_OWNER: &str = "zerx-lab";
const REPO_NAME: &str = "warp";

// GitHub requires User-Agent; explicit API version declaration avoids future default drift.
const USER_AGENT: &str = "Zap-Autoupdate";
const ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub html_url: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    /// Asset digest returned by GitHub Releases API (2024.12+) in asset metadata,
    /// formatted as `"sha256:<hex>"`. Older releases have None when this field is absent.
    #[serde(default)]
    pub digest: Option<String>,
}

impl GithubAsset {
    /// Parse the `digest` field, returning lowercase hexadecimal SHA-256 (64 characters) or None.
    /// GitHub currently returns only sha256; other algorithms are treated as None to skip validation,
    /// without granting a "green pass" based on unknown algorithms.
    pub fn sha256_hex(&self) -> Option<String> {
        let raw = self.digest.as_ref()?;
        let hex = raw.strip_prefix("sha256:")?;
        if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(hex.to_ascii_lowercase())
        } else {
            None
        }
    }
}

impl GithubRelease {
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches('v')
    }

    pub fn find_asset(&self, expected_name: &str) -> Option<&GithubAsset> {
        self.assets.iter().find(|a| a.name == expected_name)
    }
}

lazy_static! {
    /// Most recently fetched release. Written by fetch_version, read by download_update.
    /// This way, download doesn't need to re-request GitHub API and avoids races (release renewal between requests).
    static ref LATEST_RELEASE: Mutex<Option<GithubRelease>> = Mutex::new(None);
}

pub fn cached_release() -> Option<GithubRelease> {
    LATEST_RELEASE.lock().ok().and_then(|g| g.clone())
}

fn store_cached(release: GithubRelease) {
    if let Ok(mut guard) = LATEST_RELEASE.lock() {
        *guard = Some(release);
    }
}

pub async fn fetch_latest_release(client: &http_client::Client) -> Result<GithubRelease> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    log::info!("Fetching latest release from {url}");
    let release: GithubRelease = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", ACCEPT)
        .header("X-GitHub-Api-Version", API_VERSION)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .context("GitHub Releases API call failed")?
        .error_for_status()
        .context("GitHub Releases API returned non-2xx status")?
        .json()
        .await
        .context("Failed to parse GitHub Releases JSON")?;
    log::info!(
        "GitHub latest release: tag={} assets={}",
        release.tag_name,
        release.assets.len()
    );
    store_cached(release.clone());
    Ok(release)
}
