//! The app-side store for the user's **"I have read this"** marks (spec v3 §2
//! F7): owns the set, persists it as one small JSON file, and broadcasts a
//! `Changed` event so every surface showing the mark stays in sync.
//!
//! The pure store type ([`zaplex_cockpit::ReviewedSessions`]) carries the toggle
//! and the bound; this layer adds persistence and the observable-Model plumbing,
//! mirroring [`crate::cockpit::favorites::FavoritesStore`] — same load-on-`new` /
//! save-on-mutate shape, same single-file rationale (this process is the only
//! writer, so no watcher).
//!
//! Why it exists at all: the mark used to be a `HashSet` on the Conductor pane —
//! a *view*. It died when the pane closed, and worse, the pane dropped any mark
//! whose session was no longer a live row. So it vanished the moment an agent
//! finished, which is exactly when "did I already look at this?" starts to
//! matter.

use warpui::{Entity, ModelContext, SingletonEntity};
use zaplex_cockpit::ReviewedSessions;

/// Emitted whenever a mark is set or cleared. Observers re-read
/// [`ReviewedStore::contains`].
#[derive(Clone, Debug)]
pub enum ReviewedEvent {
    Changed,
}

/// Singleton owning the user's reviewed marks. Registered in `lib.rs` next to
/// the other user stores; reachable anywhere via `ReviewedStore::handle(ctx)`.
pub struct ReviewedStore {
    reviewed: ReviewedSessions,
}

impl ReviewedStore {
    /// Loads the marks file synchronously on boot — a few KB at most, like the
    /// favorites store. A missing or corrupt file starts empty (logged, never
    /// fatal): losing marks is a small annoyance, refusing to start is not.
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            reviewed: load_reviewed(),
        }
    }

    /// Has the user marked this session as read?
    ///
    /// Keyed by the session's own id, never by the Conductor's `host_key`: a
    /// daemon's `host_id` is a fresh UUID per daemon start, so a mark stored
    /// under it would silently disappear on the next restart. See
    /// [`zaplex_cockpit::reviewed`].
    pub fn contains(&self, session_id: &str) -> bool {
        self.reviewed.contains(session_id)
    }

    /// Flip the mark and persist. Returns the state afterwards.
    pub fn toggle(&mut self, session_id: &str, ctx: &mut ModelContext<Self>) -> bool {
        let now = chrono::Utc::now();
        let marked = self.reviewed.toggle(session_id, now);
        self.persist_and_notify(ctx);
        marked
    }

    fn persist_and_notify(&mut self, ctx: &mut ModelContext<Self>) {
        if let Err(err) = save_reviewed(&self.reviewed) {
            log::error!("failed to persist reviewed marks: {err:#}");
        }
        ctx.emit(ReviewedEvent::Changed);
    }
}

impl Entity for ReviewedStore {
    type Event = ReviewedEvent;
}

impl SingletonEntity for ReviewedStore {}

#[cfg(not(target_family = "wasm"))]
fn reviewed_file() -> std::path::PathBuf {
    warp_core::paths::data_dir().join("reviewed-sessions.json")
}

#[cfg(not(target_family = "wasm"))]
fn load_reviewed() -> ReviewedSessions {
    let path = reviewed_file();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|err| {
            log::warn!(
                "reviewed-marks file {} failed to parse ({err}); starting empty",
                path.display()
            );
            ReviewedSessions::default()
        }),
        // No file on first run is the normal case and says nothing.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => ReviewedSessions::default(),
        // Anything else — no permission, a bad disk — means the marks may well
        // be there and simply unreadable. Starting empty is still the only way
        // to carry on, but it must not pass for "you had none": that is how a
        // failure disguises itself as a fact.
        Err(err) => {
            log::error!(
                "reviewed-marks file {} could not be read ({err}); starting empty \
                 — marks set from now on will overwrite it",
                path.display()
            );
            ReviewedSessions::default()
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn save_reviewed(reviewed: &ReviewedSessions) -> anyhow::Result<()> {
    let path = reviewed_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(reviewed)?;
    // Write beside it, then rename over: a crash mid-write would otherwise leave
    // a half-written file, which parses as corrupt and costs every mark rather
    // than the one being set. Rename within a directory is atomic.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn load_reviewed() -> ReviewedSessions {
    ReviewedSessions::default()
}

#[cfg(target_family = "wasm")]
fn save_reviewed(_reviewed: &ReviewedSessions) -> anyhow::Result<()> {
    Ok(())
}
