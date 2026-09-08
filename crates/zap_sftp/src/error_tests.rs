use super::*;

#[test]
fn raw_sftp_no_such_file_is_classified_as_not_found() {
    let error = SftpError::Ssh2(ssh2::Error::new(ssh2::ErrorCode::SFTP(2), "no such file"));

    assert!(error.is_not_found());
}
