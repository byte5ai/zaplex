use super::{Client, headers};
use std::sync::Once;
use warp_core::channel::ChannelState;

const ZAPLEX_HEADERS: [&str; 6] = [
    headers::CLIENT_RELEASE_VERSION_HEADER_KEY,
    headers::ZAPLEX_CLIENT_ID,
    headers::ZAPLEX_OS_CATEGORY,
    headers::ZAPLEX_OS_NAME,
    headers::ZAPLEX_OS_VERSION,
    headers::ZAPLEX_OS_LINUX_KERNEL_VERSION,
];

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

fn client() -> Client {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
    Client::from_client_builder(
        reqwest::ClientBuilder::new()
            .tls_built_in_native_certs(false)
            .tls_built_in_root_certs(false)
            .no_proxy(),
    )
    .expect("test client should build")
}

fn request_headers(url: &str) -> http::HeaderMap {
    client()
        .get(url)
        .build()
        .expect("request should build")
        .wrapped
        .headers()
        .clone()
}

#[test]
fn external_provider_origin_receives_no_zaplex_headers() {
    let headers = request_headers("https://provider.example/v1/models");

    for name in ZAPLEX_HEADERS {
        assert_eq!(headers.get(name), None, "unexpected header {name}");
    }
}

#[test]
fn configured_backend_origin_is_trusted_but_other_ports_are_not() {
    let trusted_url = ChannelState::server_root_url();
    assert_eq!(
        Client::include_warp_http_headers(trusted_url.as_ref()),
        true
    );

    let mut other_origin = reqwest::Url::parse(trusted_url.as_ref()).expect("valid backend URL");
    let other_port = other_origin.port_or_known_default().unwrap_or(80) + 1;
    other_origin
        .set_port(Some(other_port))
        .expect("URL should accept a port");
    assert_eq!(
        Client::include_warp_http_headers(other_origin.as_str()),
        false
    );
}

#[test]
fn trusted_requests_only_follow_same_origin_redirects() {
    let trusted_url =
        reqwest::Url::parse(ChannelState::server_root_url().as_ref()).expect("valid backend URL");
    let same_origin = trusted_url.join("/redirected").expect("valid redirect URL");
    assert!(Client::trusted_request_may_follow_redirect(
        &trusted_url,
        &same_origin
    ));

    let external =
        reqwest::Url::parse("https://provider.example/redirected").expect("valid external URL");
    assert!(!Client::trusted_request_may_follow_redirect(
        &trusted_url,
        &external
    ));

    let mut other_port = trusted_url.clone();
    let port = other_port.port_or_known_default().unwrap_or(80) + 1;
    other_port
        .set_port(Some(port))
        .expect("URL should accept a port");
    assert!(!Client::trusted_request_may_follow_redirect(
        &trusted_url,
        &other_port
    ));
}

#[test]
fn external_requests_keep_their_normal_redirect_behavior() {
    let initial =
        reqwest::Url::parse("https://provider.example/start").expect("valid provider URL");
    let redirect = reqwest::Url::parse("https://cdn.example/result").expect("valid redirect URL");

    assert!(Client::trusted_request_may_follow_redirect(
        &initial, &redirect
    ));
}
