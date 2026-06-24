use super::*;

#[test]
fn append_returns_start_seq_and_advances_end() {
    let mut ring = OutputRing::new(1024);
    assert_eq!(ring.append(b"abc"), 0);
    assert_eq!(ring.end_seq(), 3);
    assert_eq!(ring.append(b"de"), 3);
    assert_eq!(ring.end_seq(), 5);
    assert_eq!(ring.base_seq(), 0);
    assert_eq!(ring.len(), 5);
}

#[test]
fn eviction_advances_base_seq_and_caps_len() {
    let mut ring = OutputRing::new(4);
    ring.append(b"abcd");
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.base_seq(), 0);
    ring.append(b"ef");
    assert_eq!(ring.len(), 4);
    assert_eq!(ring.base_seq(), 2);
    assert_eq!(ring.end_seq(), 6);
    let (start, bytes) = ring.replay_from(0);
    assert_eq!(start, 2); // bytes at seq 0,1 were evicted
    assert_eq!(bytes, b"cdef");
}

#[test]
fn replay_from_returns_delta() {
    let mut ring = OutputRing::new(1024);
    ring.append(b"hello world");
    let (start, bytes) = ring.replay_from(6);
    assert_eq!(start, 6);
    assert_eq!(bytes, b"world");
}

#[test]
fn replay_from_end_or_past_is_empty() {
    let mut ring = OutputRing::new(1024);
    ring.append(b"abc");
    let (start, bytes) = ring.replay_from(3);
    assert_eq!(start, 3);
    assert!(bytes.is_empty());
    let (start2, bytes2) = ring.replay_from(99);
    assert_eq!(start2, 3);
    assert!(bytes2.is_empty());
}

#[test]
fn append_larger_than_ceiling_keeps_tail() {
    let mut ring = OutputRing::new(3);
    assert_eq!(ring.append(b"abcdef"), 0);
    assert_eq!(ring.len(), 3);
    assert_eq!(ring.base_seq(), 3);
    let (start, bytes) = ring.replay_from(0);
    assert_eq!(start, 3);
    assert_eq!(bytes, b"def");
}
