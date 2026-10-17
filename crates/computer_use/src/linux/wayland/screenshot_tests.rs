use std::time::Duration;

use futures::stream;

use super::wait_for_portal_response;

#[tokio::test(start_paused = true)]
async fn request_after_timeout_can_receive_a_response() {
    let mut first_stream = stream::pending::<u32>();
    let first =
        wait_for_portal_response(&mut first_stream, Duration::from_secs(1), || async {}).await;
    assert_eq!(
        first,
        Err("Screenshot portal did not respond within 1 seconds".to_string())
    );

    let mut second_stream = stream::iter([7]);
    let second =
        wait_for_portal_response(&mut second_stream, Duration::from_secs(1), || async {}).await;
    assert_eq!(second, Ok(7));
}
