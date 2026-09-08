//! The **favorites** store — the data behind the "+" dropdown (design §10).
//!
//! A favorite is a *typed pointer* into the Conductor object tree (Host ▸ Project
//! ▸ Session ▸ saved Launch, plus GitHub instance-flows), never a copy: clicking
//! it runs the pointed-at object's default action and it duplicates no connection
//! data. This is WARP's user-owned-entries model done right and the answer to both
//! "SSH hosts in the dropdown" (favorite a host — no regression) and "register
//! once, pick everywhere" (a favorite duplicates nothing).
//!
//! Kept pure here so ordering / dedup / toggle are unit-testable without app
//! dependencies; the app-side `FavoritesStore` singleton (`app/src/cockpit/
//! favorites.rs`) owns persistence (one small JSON file) and change broadcast.
//! Targets are opaque, stable keys resolved *lazily*: a vanished target simply
//! greys out and is one-click removable (staleness is tolerated by design, so no
//! referential integrity is enforced here).

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The kind of tree object a favorite points at. `target` (on [`Favorite`]) is
/// interpreted per kind; the app resolves it to a concrete action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FavoriteKind {
    /// A registered SSH host. `target` = SSH-registry `node_id`. Default action:
    /// open a terminal on the host.
    Host,
    /// A project directory. `target` = absolute path. Default action: open the
    /// spawn card scoped to that directory.
    Project,
    /// A live agent session. `target` = session id. Default action: attach.
    Session,
    /// A saved launch/tab config. `target` = the config's stable name. Default
    /// action: launch it.
    Launch,
    /// A GitHub instance-flow. `target` = the flow key (see `github_flows`).
    /// Default action: open the spawn card carrying the flow's prompt.
    GithubFlow,
}

/// One curated favorite: a typed pointer plus a human label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub kind: FavoriteKind,
    /// The stable key of the pointed-at object (`node_id` / path / session id /
    /// config name / flow key), resolved lazily.
    pub target: String,
    /// The label shown in the dropdown. Empty falls back to `target`.
    #[serde(default)]
    pub label: String,
}

impl Favorite {
    pub fn new(kind: FavoriteKind, target: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind,
            target: target.into(),
            label: label.into(),
        }
    }

    /// Identity for dedup/toggle: two favorites are the same iff `(kind, target)`
    /// match. The label is presentation-only and is not part of identity.
    pub fn same_target(&self, kind: FavoriteKind, target: &str) -> bool {
        self.kind == kind && self.target == target
    }

    /// The text to render — the label, or the raw target when unlabeled.
    pub fn display_label(&self) -> &str {
        if self.label.is_empty() {
            &self.target
        } else {
            &self.label
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PersistedFavorite {
    Known(Favorite),
    Unknown(serde_json::Value),
}

/// The ordered, user-curated favorites store. Unknown persisted records remain
/// opaque and keep their place, while only known favorites reach the runtime
/// API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorites {
    items: Vec<Favorite>,
    persisted_items: Vec<PersistedFavorite>,
}

impl Serialize for Favorites {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let items = self
            .persisted_items
            .iter()
            .map(|item| match item {
                PersistedFavorite::Known(favorite) => {
                    serde_json::to_value(favorite).map_err(serde::ser::Error::custom)
                }
                PersistedFavorite::Unknown(value) => Ok(value.clone()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = serializer.serialize_struct("Favorites", 1)?;
        state.serialize_field("items", &items)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Favorites {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PersistedFavorites {
            #[serde(default)]
            items: Vec<serde_json::Value>,
        }

        let persisted = PersistedFavorites::deserialize(deserializer)?;
        let persisted_items = persisted
            .items
            .into_iter()
            .map(|value| match serde_json::from_value(value.clone()) {
                Ok(favorite) => PersistedFavorite::Known(favorite),
                Err(_) => PersistedFavorite::Unknown(value),
            })
            .collect();
        let mut favorites = Self {
            items: Vec::new(),
            persisted_items,
        };
        favorites.rebuild_items();
        Ok(favorites)
    }
}

impl Favorites {
    pub fn from_items(items: Vec<Favorite>) -> Self {
        let persisted_items = items
            .iter()
            .cloned()
            .map(PersistedFavorite::Known)
            .collect();
        Self {
            items,
            persisted_items,
        }
    }

    pub fn items(&self) -> &[Favorite] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Favorite hosts rendered in the "+" menu. This is a read-only projection:
    /// project, session, launch, and flow records remain in the ordered store
    /// even though they no longer appear as flat menu rows.
    pub fn host_menu_items(&self) -> impl Iterator<Item = &Favorite> {
        self.items
            .iter()
            .filter(|favorite| favorite.kind == FavoriteKind::Host)
    }

    /// Whether a favorite with this `(kind, target)` is already curated.
    pub fn contains(&self, kind: FavoriteKind, target: &str) -> bool {
        self.items.iter().any(|f| f.same_target(kind, target))
    }

    /// Append a favorite unless an identical `(kind, target)` is already present
    /// (idempotent, preserves order). Returns `true` when newly added. A re-add
    /// with a changed label refreshes the existing entry's label in place.
    pub fn add(&mut self, fav: Favorite) -> bool {
        if let Some(existing) = self.persisted_items.iter_mut().find_map(|item| match item {
            PersistedFavorite::Known(existing) if existing.same_target(fav.kind, &fav.target) => {
                Some(existing)
            }
            PersistedFavorite::Known(_) | PersistedFavorite::Unknown(_) => None,
        }) {
            existing.label = fav.label;
            self.rebuild_items();
            false
        } else {
            self.persisted_items.push(PersistedFavorite::Known(fav));
            self.rebuild_items();
            true
        }
    }

    /// Remove the favorite with this `(kind, target)`. Returns `true` if removed.
    pub fn remove(&mut self, kind: FavoriteKind, target: &str) -> bool {
        let before = self.items.len();
        self.persisted_items.retain(|item| match item {
            PersistedFavorite::Known(favorite) => !favorite.same_target(kind, target),
            PersistedFavorite::Unknown(_) => true,
        });
        self.rebuild_items();
        self.items.len() != before
    }

    /// Toggle membership: remove if present, else append. Returns the new
    /// membership state (`true` = now a favorite).
    pub fn toggle(&mut self, fav: Favorite) -> bool {
        if self.remove(fav.kind, &fav.target) {
            false
        } else {
            self.persisted_items.push(PersistedFavorite::Known(fav));
            self.rebuild_items();
            true
        }
    }

    /// Move the favorite at `from` to index `to` (drag-reorder), clamping `to`
    /// into range. No-op if `from` is out of range or equals `to`.
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.items.len() {
            return;
        }
        let to = to.min(self.items.len() - 1);
        if from == to {
            return;
        }
        let source = self
            .persisted_items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, PersistedFavorite::Known(_)))
            .nth(from)
            .map(|(index, _)| index)
            .expect("the source index was checked against known items");
        let item = self.persisted_items.remove(source);
        let known_positions: Vec<usize> = self
            .persisted_items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                PersistedFavorite::Known(_) => Some(index),
                PersistedFavorite::Unknown(_) => None,
            })
            .collect();
        let destination = known_positions.get(to).copied().unwrap_or_else(|| {
            known_positions
                .last()
                .map_or(0, |last_known| last_known + 1)
        });
        self.persisted_items.insert(destination, item);
        self.rebuild_items();
    }

    fn rebuild_items(&mut self) {
        self.items = self
            .persisted_items
            .iter()
            .filter_map(|item| match item {
                PersistedFavorite::Known(favorite) => Some(favorite.clone()),
                PersistedFavorite::Unknown(_) => None,
            })
            .collect();
    }
}

#[cfg(test)]
#[path = "favorites_tests.rs"]
mod tests;
