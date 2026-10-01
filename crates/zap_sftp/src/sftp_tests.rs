use std::io::{self, Cursor, Read, Write};
use std::path::Path;

use super::{posix_replace, MAX_RENAME_PACKET};

struct ScriptedChannel {
    responses: Cursor<Vec<u8>>,
    requests: Vec<u8>,
}

impl ScriptedChannel {
    fn new(responses: Vec<u8>) -> Self {
        Self {
            responses: Cursor::new(responses),
            requests: Vec::new(),
        }
    }
}

impl Read for ScriptedChannel {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(3);
        self.responses.read(&mut buffer[..length])
    }
}

impl Write for ScriptedChannel {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let length = buffer.len().min(2);
        self.requests.extend_from_slice(&buffer[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

fn hello(extension_version: Option<u8>) -> Vec<u8> {
    let mut bytes = vec![2, 0, 0, 0, 3];
    if let Some(version) = extension_version {
        bytes.extend_from_slice(b"\0\0\0\x18posix-rename@openssh.com\0\0\0\x01");
        bytes.push(version);
    }
    frame(&bytes)
}

fn status(id: u32, code: u32) -> Vec<u8> {
    let mut bytes = vec![101];
    bytes.extend_from_slice(&id.to_be_bytes());
    bytes.extend_from_slice(&code.to_be_bytes());
    bytes.extend_from_slice(b"\0\0\0\0\0\0\0\0"); // Empty message and language.
    frame(&bytes)
}

#[test]
fn atomic_replace_uses_exact_paths_in_posix_extension_request() {
    let mut responses = hello(Some(b'1'));
    responses.extend(status(1, 0));
    let mut channel = ScriptedChannel::new(responses);
    posix_replace(
        &mut channel,
        Path::new("-source;$x"),
        Path::new("target's name"),
    )
    .unwrap();

    let mut expected = frame(&[1, 0, 0, 0, 3]);
    let mut request = b"\xc8\0\0\0\x01\0\0\0\x18posix-rename@openssh.com".to_vec();
    request.extend_from_slice(b"\0\0\0\x0a-source;$x");
    request.extend_from_slice(b"\0\0\0\x0dtarget's name");
    expected.extend(frame(&request));
    assert_eq!(channel.requests, expected);
}

#[test]
fn atomic_replace_requires_advertised_exact_extension_version_before_mutation() {
    for version in [None, Some(b'0'), Some(b'2')] {
        let mut channel = ScriptedChannel::new(hello(version));
        assert!(posix_replace(&mut channel, Path::new("source"), Path::new("target")).is_err());
        assert_eq!(channel.requests, frame(&[1, 0, 0, 0, 3]));
    }
}

#[test]
fn atomic_replace_rejects_failed_or_mismatched_acknowledgement() {
    for (id, code) in [(1, 4), (2, 0), (1, 8)] {
        let mut responses = hello(Some(b'1'));
        responses.extend(status(id, code));
        let mut channel = ScriptedChannel::new(responses);
        assert!(posix_replace(&mut channel, Path::new("source"), Path::new("target")).is_err());
        assert!(
            channel.requests.len() > 9,
            "the rejection follows the mutation request"
        );
    }
}

#[test]
fn atomic_replace_rejects_malformed_hello_without_sending_mutation() {
    let mut oversized = ((MAX_RENAME_PACKET + 1) as u32).to_be_bytes().to_vec();
    oversized.push(2);
    let mut truncated_string = vec![2, 0, 0, 0, 3];
    truncated_string.extend_from_slice(&u32::MAX.to_be_bytes());
    for response in [
        Vec::new(),
        vec![0, 0, 0, 0],
        oversized,
        frame(&[2, 0, 0]),
        frame(&[101, 0, 0, 0, 3]),
        frame(&[2, 0, 0, 0, 4]),
        frame(&truncated_string),
    ] {
        let mut channel = ScriptedChannel::new(response);
        assert!(posix_replace(&mut channel, Path::new("source"), Path::new("target")).is_err());
        assert_eq!(channel.requests, frame(&[1, 0, 0, 0, 3]));
    }
}

#[test]
fn atomic_replace_never_treats_truncated_status_as_success() {
    let complete = status(1, 0);
    for length in 0..complete.len() {
        let mut responses = hello(Some(b'1'));
        responses.extend_from_slice(&complete[..length]);
        let mut channel = ScriptedChannel::new(responses);
        assert!(posix_replace(&mut channel, Path::new("source"), Path::new("target")).is_err());
    }
    let mut invalid = status(1, 0);
    invalid[4] = 102; // An extended reply cannot acknowledge rename.
    let mut responses = hello(Some(b'1'));
    responses.extend(invalid);
    assert!(posix_replace(
        &mut ScriptedChannel::new(responses),
        Path::new("source"),
        Path::new("target")
    )
    .is_err());
}

#[test]
fn atomic_replace_rejects_invalid_or_oversized_paths_before_any_request() {
    let oversized = "x".repeat(MAX_RENAME_PACKET);
    for path in ["", "invalid\0path", oversized.as_str()] {
        let mut channel = ScriptedChannel::new(Vec::new());
        assert!(posix_replace(&mut channel, Path::new(path), Path::new("target")).is_err());
        assert!(channel.requests.is_empty());
    }
}

#[test]
fn atomic_replace_propagates_request_write_failure() {
    struct FailedWriter;
    impl Read for FailedWriter {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            panic!("a failed INIT must not read a response");
        }
    }
    impl Write for FailedWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "fixture"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    assert!(posix_replace(&mut FailedWriter, Path::new("source"), Path::new("target")).is_err());
}
