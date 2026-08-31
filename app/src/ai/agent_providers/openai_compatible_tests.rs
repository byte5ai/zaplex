use http_client::headers;
use mockito::{Matcher, Server};

use super::fetch_openai_compatible_models;

#[tokio::test]
async fn external_model_origin_receives_no_zaplex_metadata_headers() {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("GET", "/models")
        .match_header(headers::CLIENT_RELEASE_VERSION_HEADER_KEY, Matcher::Missing)
        .match_header("X-Zap-Client-ID", Matcher::Missing)
        .match_header("X-Zap-OS-Category", Matcher::Missing)
        .match_header("X-Zap-OS-Name", Matcher::Missing)
        .match_header("X-Zap-OS-Version", Matcher::Missing)
        .match_header("X-Zap-OS-Linux-Kernel-Version", Matcher::Missing)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"data":[{"id":"model-a"}]}"#)
        .create_async()
        .await;

    let models =
        fetch_openai_compatible_models(http_client::Client::new_for_test(), &server.url(), None)
            .await
            .expect("model fetch should succeed");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "model-a");
    mock.assert_async().await;
}
