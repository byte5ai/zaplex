//! Global HTTP proxy configuration.
//!
//! See Issue #72: Zap needs a globally-configurable proxy setting that uniformly covers
//! all outbound HTTP requests (BYOP model list fetching, autoupdate, conversation loading, etc.).
//!
//! Design points:
//! - Three modes in [`ProxyMode`]: `System` / `Custom` / `Off`.
//! - `System` falls back to reqwest's default behavior; the workspace's reqwest has
//!   `system-proxy` + `macos-system-configuration` features enabled, so on macOS it reads
//!   SystemConfiguration, on Windows reads WinINET, on Linux reads `HTTP_PROXY` etc. env vars;
//!   no custom implementation needed.
//! - `Custom` explicitly specifies URL / basic auth / no_proxy list.
//! - `Off` calls [`reqwest::ClientBuilder::no_proxy`], completely disabling proxy (including env vars).
//!
//! The application injects config via [`set_global_proxy_config`] at startup / on settings change;
//! all subsequent [`crate::Client::new`] calls read this global value and apply it to reqwest.
//!
//! reqwest does not support runtime proxy switching on an already-constructed `Client`,
//! so callers must rebuild the Client instance after changing settings (e.g. `AutoupdateState::new(http_client::Client::new())`).

use std::sync::{OnceLock, RwLock};

/// Global proxy mode.
///
/// Default is `Off`: prevents `Client` instances constructed during cold startup
/// (before app-level settings are injected) from accidentally using system proxies detected by reqwest.
/// app::ProxyMode defaults to the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyMode {
    /// Disable proxy, including environment variables. Default.
    #[default]
    Off,
    /// Fully follow system / environment variables (reqwest default behavior).
    System,
    /// Use the proxy explicitly configured in [`ProxyConfig::url`].
    Custom,
}

impl ProxyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyMode::System => "system",
            ProxyMode::Custom => "custom",
            ProxyMode::Off => "off",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "system" => ProxyMode::System,
            "custom" => ProxyMode::Custom,
            // off / disabled / none / unknown all fall back to Off (default) to avoid accidentally using system proxy.
            _ => ProxyMode::Off,
        }
    }
}

/// Parsed global proxy configuration.
///
/// `username` is stored plaintext in settings.toml; `password` is stored separately
/// via `managed_secrets` (same pattern as BYOP API key) and injected into [`Self::password`] by callers before assembling this struct.
#[derive(Clone, Debug, Default)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    /// Example: `http://proxy.corp:8080`. Only effective in [`ProxyMode::Custom`] mode.
    pub url: String,
    pub username: String,
    pub password: String,
    /// Comma-separated list of hosts; empty string means no exceptions.
    pub no_proxy: String,
}

impl ProxyConfig {
    /// Apply this configuration to `reqwest::ClientBuilder`.
    ///
    /// On error (Custom mode but URL is invalid), log a warning and fall back to reqwest's default behavior
    /// without panicking in `Client::new()`.
    pub fn apply(&self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        match self.mode {
            ProxyMode::System => builder,
            ProxyMode::Off => builder.no_proxy(),
            ProxyMode::Custom => {
                let trimmed = self.url.trim();
                if trimmed.is_empty() {
                    log::warn!("HTTP proxy set to Custom but URL is empty; falling back to reqwest default (reading system proxy)");
                    return builder;
                }

                let proxy_result = reqwest::Proxy::all(trimmed);
                let mut proxy = match proxy_result {
                    Ok(p) => p,
                    Err(err) => {
                        log::warn!("HTTP proxy URL '{trimmed}' is invalid ({err}); falling back to reqwest default");
                        return builder;
                    }
                };

                if !self.username.is_empty() || !self.password.is_empty() {
                    proxy = proxy.basic_auth(&self.username, &self.password);
                }

                if !self.no_proxy.trim().is_empty() {
                    if let Some(no_proxy) = reqwest::NoProxy::from_string(self.no_proxy.trim()) {
                        proxy = proxy.no_proxy(Some(no_proxy));
                    }
                }

                builder = builder.proxy(proxy);
                builder
            }
        }
    }
}

static GLOBAL_PROXY_CONFIG: OnceLock<RwLock<ProxyConfig>> = OnceLock::new();

fn slot() -> &'static RwLock<ProxyConfig> {
    GLOBAL_PROXY_CONFIG.get_or_init(|| RwLock::new(ProxyConfig::default()))
}

/// Install a new global proxy configuration.
///
/// Only affects `Client` instances constructed after this call. Since `reqwest::Client` cannot
/// switch proxies once constructed, the application must rebuild all shared Client instances after changing settings.
pub fn set_global_proxy_config(cfg: ProxyConfig) {
    let lock = slot();
    if let Ok(mut guard) = lock.write() {
        *guard = cfg;
    } else {
        log::error!("Failed to write global HTTP proxy config: RwLock is poisoned");
    }
}

/// Read the current global proxy configuration (returns default if not set).
pub fn current_proxy_config() -> ProxyConfig {
    let lock = slot();
    match lock.read() {
        Ok(guard) => guard.clone(),
        Err(err) => {
            log::error!("Failed to read global HTTP proxy config: RwLock is poisoned ({err})");
            ProxyConfig::default()
        }
    }
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
