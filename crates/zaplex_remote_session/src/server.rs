//! Daemon-side session host: PTY ownership + shell spawn + output ring buffer
//! + session registry.
//!
//! Stage 1: the daemon owns PTYs, spawns user shells, buffers their output in a
//! per-session [`output_ring::OutputRing`], and streams it to the attached
//! connection. See
//! `docs/superpowers/specs/2026-06-24-stage1-session-host-design.md`.

pub mod output_ring;
