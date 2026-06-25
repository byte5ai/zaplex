//! Bounded per-session output ring buffer.
//!
//! Stores the most recent PTY output bytes up to a byte ceiling, tracking a
//! monotonically increasing byte offset (`seq`) so a reconnecting client can
//! replay everything it has not yet consumed. `seq` is a byte offset into the
//! session's lifetime output stream: the first byte ever written has seq 0, and
//! `end_seq()` is the seq just past the last byte written.

use std::collections::VecDeque;

/// A bounded ring of recent session output bytes with replay support.
pub struct OutputRing {
    buf: VecDeque<u8>,
    max_bytes: usize,
    /// Seq (byte offset) of the oldest byte still retained in `buf`.
    base_seq: u64,
}

impl OutputRing {
    /// Creates a ring that retains at most `max_bytes` of the most recent output
    /// (clamped to at least 1 byte).
    pub fn new(max_bytes: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            max_bytes: max_bytes.max(1),
            base_seq: 0,
        }
    }

    /// Seq just past the last byte written — i.e. the total number of bytes ever
    /// appended over the session's lifetime.
    pub fn end_seq(&self) -> u64 {
        self.base_seq + self.buf.len() as u64
    }

    /// Seq of the oldest byte still available for replay.
    pub fn base_seq(&self) -> u64 {
        self.base_seq
    }

    /// Number of bytes currently retained.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the ring currently holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Appends `data` and returns the seq at which `data` starts (the seq of its
    /// first byte). Evicts the oldest bytes once the ceiling is exceeded,
    /// advancing `base_seq` accordingly.
    pub fn append(&mut self, data: &[u8]) -> u64 {
        let start_seq = self.end_seq();
        self.buf.extend(data.iter().copied());
        if self.buf.len() > self.max_bytes {
            let overflow = self.buf.len() - self.max_bytes;
            self.buf.drain(..overflow);
            self.base_seq += overflow as u64;
        }
        start_seq
    }

    /// Returns the retained bytes from `from_seq` onward together with the seq of
    /// the first returned byte — which is greater than `from_seq` when the
    /// requested start has already been evicted. When `from_seq` is at or past
    /// the end, returns an empty vec at `end_seq()`.
    pub fn replay_from(&self, from_seq: u64) -> (u64, Vec<u8>) {
        let end = self.end_seq();
        if from_seq >= end {
            return (end, Vec::new());
        }
        let start = from_seq.max(self.base_seq);
        let offset = (start - self.base_seq) as usize;
        let bytes = self.buf.iter().skip(offset).copied().collect();
        (start, bytes)
    }
}

#[cfg(test)]
#[path = "output_ring_tests.rs"]
mod tests;
