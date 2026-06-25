//! Terminal backed by a *daemon-hosted* PTY session.
//!
//! This is the client side of the native persistent remote-session layer: the
//! remote daemon owns the PTY and an output replay buffer, and streams its
//! output over the remote-server protocol. The client is an attached *view* of
//! that session, so the session survives an SSH/transport drop and can be
//! reattached.
//!
//! It is deliberately a sibling of [`super::remote_tty`] rather than a fork of
//! it: both reuse the same shared terminal-manager helpers, and only the
//! transport (the [`event_loop`]) differs. [`super::local_tty`] (localhost and
//! plain-vanilla SSH) stays the default and is untouched by this module.

mod event_loop;
mod terminal_manager;

pub use terminal_manager::{OpenSessionParams, TerminalManager};
