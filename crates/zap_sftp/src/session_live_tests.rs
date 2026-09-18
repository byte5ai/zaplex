use super::{AuthMethod, SftpSession};
use ssh2::HostKeyType;
use std::path::PathBuf;

/// The isolated CI server offers ECDSA and Ed25519, while known_hosts pins only
/// Ed25519. A client that negotiates first and reads pins later fails this test.
#[test]
#[ignore = "requires the isolated OpenSSH fixture in live-sftp-safety.yml"]
fn live_sftp_prefers_known_host_key() {
    let host = std::env::var("ZAPLEX_LIVE_SFTP_HOST").unwrap();
    let port = std::env::var("ZAPLEX_LIVE_SFTP_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let username = std::env::var("ZAPLEX_LIVE_SFTP_USERNAME").unwrap();
    let key_path = PathBuf::from(std::env::var("ZAPLEX_LIVE_SFTP_KEY_PATH").unwrap());
    let connection = SftpSession::connect(
        &host,
        port,
        &username,
        AuthMethod::PublicKey {
            key_path,
            passphrase: None,
        },
        None,
    )
    .expect("the unchanged known server key must connect without replacement");
    assert!(matches!(
        connection.session.host_key().unwrap().1,
        HostKeyType::Ed25519
    ));
    connection
        .sftp()
        .expect("the verified connection must open SFTP");
}
