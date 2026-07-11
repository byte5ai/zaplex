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
}

impl FavoritesStore {
    /// Loads the favorites file synchronously on boot — it is a few KB at most,
    /// so no need for a background load (mirrors `WarpConfig`'s synchronous theme
    /// load). A missing or corrupt file starts empty (logged, never fatal).
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            favorites: load_favorites(),
        }
    }

    /// The curated favorites, in the user's order.
    pub fn items(&self) -> &[Favorite] {
        &self.favorites.items
    }

    pub fn is_empty(&self) -> bool {
        self.favorites.is_empty()
    }

    /// Whether `(kind, target)` is already curated (drives the ★ filled/empty
    /// state on a tree node).
    pub fn contains(&self, kind: FavoriteKind, target: &str) -> bool {
        self.favorites.contains(kind, target)
    }

    /// Toggle a favorite (the ★ affordance on a tree node). Returns the new
    /// membership state; persists and broadcasts.
    pub fn toggle(&mut self, fav: Favorite, ctx: &mut ModelContext<Self>) -> bool {
        let now = self.favorites.toggle(fav);
        self.persist_and_notify(ctx);
        now
    }

    /// Add a favorite ("＋ Add favorite…"). Idempotent; persists + broadcasts
    /// only when something actually changed (a new entry or a refreshed label).
    pub fn add(&mut self, fav: Favorite, ctx: &mut ModelContext<Self>) {
        let added = self.favorites.add(fav);
        // `add` returns false when the entry already existed, but it may still
        // have refreshed the label — persist unconditionally so a rename sticks.
        let _ = added;
        self.persist_and_notify(ctx);
    }

    /// Remove a favorite (the one-click remove on a stale entry, or un-starring).
    pub fn remove(&mut self, kind: FavoriteKind, target: &str, ctx: &mut ModelContext<Self>) {
        if self.favorites.remove(kind, target) {
            self.persist_and_notify(ctx);
        }
    }

    fn persist_and_notify(&mut self, ctx: &mut ModelContext<Self>) {
        if let Err(err) = save_favorites(&self.favorites) {
            log::error!("failed to persist favorites: {err:#}");
        }
        ctx.emit(FavoritesEvent::Changed);
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
fn load_favorites() -> Favorites {
    let path = favorites_file();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|err| {
            log::warn!(
                "favorites file {} failed to parse ({err}); starting empty",
                path.display()
            );
            Favorites::default()
        }),
        // Absent file on first run is normal → empty store.
        Err(_) => Favorites::default(),
    }
}

#[cfg(not(target_family = "wasm"))]
fn save_favorites(favorites: &Favorites) -> anyhow::Result<()> {
    let path = favorites_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(favorites)?;
    std::fs::write(&path, json)?;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn load_favorites() -> Favorites {
    Favorites::default()
}

#[cfg(target_family = "wasm")]
fn save_favorites(_favorites: &Favorites) -> anyhow::Result<()> {
    Ok(())
}
