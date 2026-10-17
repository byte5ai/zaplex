//! Unit tests for the `proxy` module.
//!
//! reqwest 0.12 has no public API to query registered `Proxy` on `ClientBuilder`,
//! so we can only do minimal verification from observable behavior (whether `Client`
//! construction after `apply` succeeds). Finer-grained "whether it actually routes through proxy"
//! testing is left to integration tests (requires local mitm setup).
//!
//! Note: reqwest with `rustls-tls-native-roots-no-provider` features requires a global crypto
//! provider to be installed before `.build()`, else it panics. Production code installs this in
//! `app/src/lib.rs::init_common`; unit tests need to install it themselves.

use super::*;
use std::sync::Once;

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

/// Call before running tests that use reqwest `.build()`; only takes effect on first call.
fn ensure_crypto_provider() {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Build a ClientBuilder with native CA loading disabled to avoid build failures
/// in environments where system certificates are hard to obtain.
fn test_builder() -> reqwest::ClientBuilder {
    ensure_crypto_provider();
    reqwest::ClientBuilder::new()
        .tls_built_in_native_certs(false)
        .tls_built_in_root_certs(false)
}

#[test]
fn proxy_mode_from_str_lenient_handles_variants() {
    assert_eq!(ProxyMode::from_str_lenient("system"), ProxyMode::System);
    assert_eq!(ProxyMode::from_str_lenient("SYSTEM"), ProxyMode::System);
    assert_eq!(ProxyMode::from_str_lenient("custom"), ProxyMode::Custom);
    assert_eq!(ProxyMode::from_str_lenient("off"), ProxyMode::Off);
    assert_eq!(ProxyMode::from_str_lenient("disabled"), ProxyMode::Off);
    assert_eq!(ProxyMode::from_str_lenient("none"), ProxyMode::Off);
    // Unknown values fall back to Off, consistent with default, avoids accidental system proxy use.
    assert_eq!(ProxyMode::from_str_lenient("wat"), ProxyMode::Off);
}

#[test]
fn proxy_mode_as_str_roundtrip() {
    for mode in [ProxyMode::System, ProxyMode::Custom, ProxyMode::Off] {
        let s = mode.as_str();
        assert_eq!(ProxyMode::from_str_lenient(s), mode);
    }
}

#[test]
fn apply_off_disables_proxy_without_error() {
    let cfg = ProxyConfig {
        mode: ProxyMode::Off,
        ..Default::default()
    };
    let builder = cfg.apply(test_builder());
    let _client = builder.build().expect("Off mode should build successfully");
}

#[test]
fn apply_custom_with_valid_url_succeeds() {
    let cfg = ProxyConfig {
        mode: ProxyMode::Custom,
        url: "http://proxy.corp:8080".to_string(),
        ..Default::default()
    };
    let builder = cfg.apply(test_builder());
    let _client = builder
        .build()
        .expect("Custom mode + valid URL should build successfully");
}

#[test]
fn apply_custom_with_basic_auth_succeeds() {
    let cfg = ProxyConfig {
        mode: ProxyMode::Custom,
        url: "http://proxy.corp:8080".to_string(),
        username: "alice".to_string(),
        password: "s3cret".to_string(),
        ..Default::default()
    };
    let builder = cfg.apply(test_builder());
    let _client = builder.build().expect("Custom + auth should build successfully");
}
