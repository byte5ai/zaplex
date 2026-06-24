//! A single `Mutex<SqliteConnection>` shared across the entire process for use by the SSH manager.
//!
//! Status: openWarp's main write connection runs in a dedicated write thread (see `app/src/persistence/sqlite.rs`)
//! and is processed asynchronously via `ModelEvent` channel. Integrating the SSH manager into that event bus
//! would require 6+ enum variants + cross-crate type exposure, which is too costly.
//!
//! Alternative approach: **SQLite WAL mode natively supports multiple write connections** (writes are mutually exclusive
//! but with busy_timeout retry). Here we open an independent write connection whose behavior is completely localized
//! to this crate. SSH manager write operations are user-driven (create/delete nodes) with very low frequency,
//! so conflicts with the main write thread are negligible.
//!
//! The path is passed in by the caller during initialization (`set_database_path`), avoiding direct dependency
//! by this crate on the app layer's `database_file_path()`. If no path is provided, `with_conn` returns `Err(NotInitialized)`.

use anyhow::{Result, anyhow};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DB_PATH: OnceLock<PathBuf> = OnceLock::new();
static CONN: OnceLock<Mutex<SqliteConnection>> = OnceLock::new();

/// Called once by app startup with the sqlite db file path. Repeated calls are ignored
/// (OnceLock semantics).
pub fn set_database_path(path: PathBuf) {
    let _ = DB_PATH.set(path);
}

fn open() -> Result<SqliteConnection> {
    let path = DB_PATH
        .get()
        .ok_or_else(|| anyhow!("warp_ssh_manager::db: database path not initialized"))?;
    let url = path.to_string_lossy();
    let mut conn = SqliteConnection::establish(&url)?;
    conn.batch_execute(
        "PRAGMA foreign_keys = ON; \
         PRAGMA busy_timeout = 2000; \
         PRAGMA journal_mode = WAL;",
    )?;
    Ok(conn)
}

/// Execute closure within lock. Opens connection lazily on first call; reuses on subsequent calls.
pub fn with_conn<R>(f: impl FnOnce(&mut SqliteConnection) -> Result<R>) -> Result<R> {
    let mtx = CONN.get_or_init(|| Mutex::new(open().expect("warp_ssh_manager db open")));
    let mut guard = mtx
        .lock()
        .map_err(|_| anyhow!("warp_ssh_manager db mutex poisoned"))?;
    f(&mut guard)
}
