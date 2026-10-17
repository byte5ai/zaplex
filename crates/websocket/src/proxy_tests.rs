use std::sync::Mutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

/// Guard that ensures proxy-related env vars are cleaned up after each test.
/// Tests that manipulate env vars must hold this lock to avoid races.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn clear_proxy_env() {
    // Environment-resolution tests explicitly exercise System mode; production defaults to Off.
    set_global_proxy_config(ProxyConfig {
        mode: ProxyMode::System,
        ..Default::default()
    });
    for var in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        env::remove_var(var);
    }
}

fn wss_uri(host: &str) -> http::Uri {
    format!("wss://{host}").parse().unwrap()
}

fn ws_uri(host: &str) -> http::Uri {
    format!("ws://{host}").parse().unwrap()
}

fn resolved_proxy_tls(host: &str) -> Option<ProxyInfo> {
    resolve_proxy(&wss_uri(host)).expect("proxy resolution should succeed")
}

fn resolved_proxy_plain(host: &str) -> Option<ProxyInfo> {
    resolve_proxy(&ws_uri(host)).expect("proxy resolution should succeed")
}

// -- resolve_proxy tests --

#[test]
fn resolve_proxy_returns_none_when_no_env_vars_set() {
    let _lock = ENV_LOCK.lock();
    clear_proxy_env();
    assert!(resolved_proxy_tls("example.com").is_none());
    assert!(resolved_proxy_plain("example.com").is_none());
}

#[test]
fn resolve_proxy_reads_lowercase_env_vars() {
    let _lock = ENV_LOCK.lock();
    clear_proxy_env();
    env::set_var("https_proxy", "http://lower.corp:4444");

    let info = resolved_proxy_tls("example.com").expect("should resolve from lowercase");
    assert_eq!(info.host, "lower.corp");
    assert_eq!(info.port, 4444);
    clear_proxy_env();
}

#[test]
fn resolve_proxy_rejects_https_proxy_urls() {
    let _lock = ENV_LOCK.lock();
    clear_proxy_env();
    env::set_var("HTTPS_PROXY", "https://proxy.corp:443");

    let err = resolve_proxy(&wss_uri("example.com")).expect_err("https proxy URLs should fail");
    let err_msg = format!("{err:#}");
    assert!(err_msg.contains("Invalid proxy URL configured in HTTPS_PROXY"));
    assert!(err_msg.contains("HTTPS proxy URLs are not supported"));
    clear_proxy_env();
}

// -- parse_proxy_url tests --

#[test]
fn parse_proxy_url_with_credentials() {
    let info = parse_proxy_url("http://user:pass@proxy.corp:3128").expect("should parse");
    assert_eq!(info.host, "proxy.corp");
    assert_eq!(info.port, 3128);
    let decoded = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(info.basic_auth.as_ref().unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(decoded, "user:pass");
}

#[test]
fn parse_proxy_url_decodes_percent_encoded_credentials() {
    let info = parse_proxy_url("http://user%40name:p%3Ass@proxy.corp:3128").expect("should parse");
    let decoded = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(info.basic_auth.as_ref().expect("basic auth should exist"))
            .expect("basic auth should be valid base64"),
    )
    .expect("decoded basic auth should be valid UTF-8");
    assert_eq!(decoded, "user@name:p:ss");
}

// -- connect_via_proxy integration test with mock proxy --

#[tokio::test]
async fn connect_via_proxy_sends_auth_header() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let proxy_info = ProxyInfo {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        basic_auth: Some(BASE64.encode("user:secret")),
    };

    let mock_proxy = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        request
    });

    let target_uri: http::Uri = "wss://host.example.com:8443".parse().unwrap();
    let result = connect_via_proxy(&proxy_info, &target_uri).await;
    assert!(result.is_ok());

    let request_sent = mock_proxy.await.unwrap();
    let expected_auth = format!(
        "proxy-authorization: Basic {}",
        BASE64.encode("user:secret")
    );
    assert!(
        request_sent.contains(&expected_auth),
        "Request should contain auth header: {request_sent}"
    );
}
