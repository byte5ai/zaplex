//! The app-side **favorites** singleton: owns the user's curated favorites list
//! (the "+" dropdown, design §10), persists it as one small JSON file, and
//! broadcasts a `Changed` event so the dropdown and the Conductor tree stay in
//! sync.
//!
//! The pure store type ([`zaplex_cockpit::Favorites`]) carries ordering / dedup /
//! toggle; this layer adds persistence and the observable-Model plumbing. It
//! mirrors [`crate::user_config::WarpConfig`]'s load-on-`new` / save-on-mutate
//! shape, but for a *single* file rather than a directory of items — favorites is
//! one ordered list, so a file watcher is unnecessary (this process is the only
//! writer). File I/O is `#[cfg(not(wasm))]` like the rest of the cockpit's disk
//! access; on wasm the store is in-memory and empty.

use warpui::{Entity, ModelContext, SingletonEntity};
use zaplex_cockpit::{Favorite, FavoriteKind, Favorites};

#[cfg(not(target_family = "wasm"))]
use anyhow::{bail, Context as _};
#[cfg(not(target_family = "wasm"))]
use std::io::Write as _;
#[cfg(not(target_family = "wasm"))]
use tempfile::NamedTempFile;

/// Whether the on-disk source may be replaced.
///
/// A missing or valid file is writable. Corrupt or unreadable input is
/// protected for the lifetime of this store so an empty fallback can never
/// destroy the user's last recoverable copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FavoritesFileState {
    Missing,
    Loaded,
    Protected,
    #[cfg(test)]
    InMemory,
}

/// Emitted whenever the curated favorites change (add / remove / toggle /
/// reorder). Observers re-read [`FavoritesStore::items`].
#[derive(Clone, Debug)]
pub enum FavoritesEvent {
    Changed,
}

/// Singleton owning the user's favorites. Registered in `lib.rs` next to the
/// other user stores; reachable anywhere via `FavoritesStore::handle(ctx)`.
pub struct FavoritesStore {
    favorites: Favorites,
    file_state: FavoritesFileState,
}

impl FavoritesStore {
    /// Loads the favorites file synchronously on boot — it is a few KB at most,
    /// so no need for a background load (mirrors `WarpConfig`'s synchronous theme
    /// load). A missing file starts empty. Corrupt or unreadable input also
    /// degrades to an empty in-memory view, but is protected from overwrite.
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        let (favorites, file_state) = load_favorites();
        Self {
            favorites,
            file_state,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(_: &mut ModelContext<Self>) -> Self {
        Self {
            favorites: Favorites::default(),
            file_state: FavoritesFileState::InMemory,
        }
    }

    /// The curated favorites, in the user's order.
    pub fn items(&self) -> &[Favorite] {
        self.favorites.items()
    }

    /// Favorite hosts projected into the "+" menu. Other favorite kinds remain
    /// persisted and available to future surfaces, but are not flat menu rows.
    pub fn host_menu_items(&self) -> impl Iterator<Item = &Favorite> {
        self.favorites.host_menu_items()
    }

    pub fn is_empty(&self) -> bool {
        self.favorites.is_empty()
    }

    /// Whether persistence is fail-closed because the source was corrupt,
    /// unreadable, or a write failed. UI surfaces this instead of pretending an
    /// empty fallback is a healthy store.
    pub fn persistence_is_protected(&self) -> bool {
        self.file_state == FavoritesFileState::Protected
    }

    /// Whether `(kind, target)` is already curated (drives the ★ filled/empty
    /// state on a tree node).
    pub fn contains(&self, kind: FavoriteKind, target: &str) -> bool {
        self.favorites.contains(kind, target)
    }

    /// Toggle a favorite (the ★ affordance on a tree node). Returns the new
    /// membership state; persists and broadcasts.
    pub fn toggle(&mut self, fav: Favorite, ctx: &mut ModelContext<Self>) -> bool {
        let previous = self.favorites.clone();
        let previous_membership = previous.contains(fav.kind, &fav.target);
        let now = self.favorites.toggle(fav);
        if self.persist_and_notify(previous, ctx) {
            now
        } else {
            previous_membership
        }
    }

    /// Add a favorite ("＋ Add favorite…"). Idempotent; persists + broadcasts
    /// only when something actually changed (a new entry or a refreshed label).
    pub fn add(&mut self, fav: Favorite, ctx: &mut ModelContext<Self>) {
        let previous = self.favorites.clone();
        self.favorites.add(fav);
        if self.favorites != previous {
            self.persist_and_notify(previous, ctx);
        }
    }

    /// Remove a favorite (the one-click remove on a stale entry, or un-starring).
    pub fn remove(&mut self, kind: FavoriteKind, target: &str, ctx: &mut ModelContext<Self>) {
        let previous = self.favorites.clone();
        if self.favorites.remove(kind, target) {
            self.persist_and_notify(previous, ctx);
        }
    }

    fn persist_and_notify(&mut self, previous: Favorites, ctx: &mut ModelContext<Self>) -> bool {
        #[cfg(test)]
        if self.file_state == FavoritesFileState::InMemory {
            ctx.emit(FavoritesEvent::Changed);
            return true;
        }
        if let Err(err) = save_favorites(&self.favorites, self.file_state) {
            self.favorites = previous;
            self.file_state = FavoritesFileState::Protected;
            log::error!("failed to persist favorites: {err:#}");
            ctx.emit(FavoritesEvent::Changed);
            return false;
        }
        self.file_state = FavoritesFileState::Loaded;
        ctx.emit(FavoritesEvent::Changed);
        true
    }
}

impl Entity for FavoritesStore {
    type Event = FavoritesEvent;
}

impl SingletonEntity for FavoritesStore {}

#[cfg(not(target_family = "wasm"))]
fn favorites_file() -> std::path::PathBuf {
    warp_core::paths::data_dir().join("favorites.json")
}

#[cfg(not(target_family = "wasm"))]
fn load_favorites_from(path: &std::path::Path) -> (Favorites, FavoritesFileState) {
    match std::fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(favorites) => (favorites, FavoritesFileState::Loaded),
            Err(err) => {
                log::warn!(
                    "favorites file {} failed to parse ({err}); protecting it from overwrite",
                    path.display()
                );
                (Favorites::default(), FavoritesFileState::Protected)
            }
        },
        // Absent file on first run is normal → empty store.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            (Favorites::default(), FavoritesFileState::Missing)
        }
        Err(err) => {
            log::warn!(
                "favorites file {} could not be read ({err}); protecting it from overwrite",
                path.display()
            );
            (Favorites::default(), FavoritesFileState::Protected)
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn load_favorites() -> (Favorites, FavoritesFileState) {
    load_favorites_from(&favorites_file())
}

#[cfg(not(target_family = "wasm"))]
fn save_favorites_to(
    path: &std::path::Path,
    favorites: &Favorites,
    state: FavoritesFileState,
) -> anyhow::Result<()> {
    if state == FavoritesFileState::Protected {
        bail!(
            "refusing to overwrite unreadable or corrupt favorites file {}",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(favorites)?;
    let parent = path
        .parent()
        .with_context(|| format!("favorites path {} has no parent", path.display()))?;
    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file for {}", path.display()))?;
    temp.write_all(json.as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    temp.persist(path)
        .map_err(|err| anyhow::Error::from(err.error))
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
fn save_favorites(favorites: &Favorites, state: FavoritesFileState) -> anyhow::Result<()> {
    save_favorites_to(&favorites_file(), favorites, state)
}

#[cfg(target_family = "wasm")]
fn load_favorites() -> (Favorites, FavoritesFileState) {
    (Favorites::default(), FavoritesFileState::Missing)
}

#[cfg(target_family = "wasm")]
fn save_favorites(_favorites: &Favorites, _state: FavoritesFileState) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "favorites_tests.rs"]
mod tests;
