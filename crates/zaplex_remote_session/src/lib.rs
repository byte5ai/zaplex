//! `zaplex_remote_session` — zaplex's native persistent remote-session layer.
//!
//! See the implementation plan at
//! `docs/superpowers/plans/2026-06-24-native-remote-session-layer.md`.
//! This crate carries both the client- and daemon-side session logic, gated
//! behind cargo features; the transport-agnostic data types shared by both
//! sides live in [`types`].
//!
//! Stage 0 (this commit) only sets up the scaffold: shared types + capability
//! constants + placeholder modules. It contains no PTY/transport implementation
//! and has zero effect on existing behaviour. Later stages fill in the
//! [`server`] and [`client`] modules.

pub mod types;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "server")]
pub mod server;
