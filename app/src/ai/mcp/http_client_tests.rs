use std::{collections::HashMap, sync::Once};

use ::http_client::{ProxyConfig, ProxyMode};

use super::build_client_with_headers_and_proxy;

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

fn ensure_crypto_provider() {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[test]
fn mcp_client_accepts_every_global_proxy_mode() {
    ensure_crypto_provider();
    let headers = HashMap::from([("x-test".to_string(), "mcp".to_string())]);
    let configs = [
        ProxyConfig {
            mode: ProxyMode::System,
            ..Default::default()
        },
        ProxyConfig {
            mode: ProxyMode::Custom,
            url: "http://proxy.example:8080".to_string(),
            username: "alice".to_string(),
            password: "secret".to_string(),
            no_proxy: "localhost,.internal".to_string(),
        },
        ProxyConfig {
            mode: ProxyMode::Off,
            ..Default::default()
        },
    ];

    for config in configs {
        build_client_with_headers_and_proxy(&headers, config)
            .expect("MCP client should accept the configured proxy mode");
    }
}
