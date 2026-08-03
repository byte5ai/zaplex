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

use serde::{Deserialize, Serialize};

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

/// The ordered, user-curated favorites store. Order is the user's curation order
/// (append-on-add), preserved verbatim on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorites {
    #[serde(default)]
    pub items: Vec<Favorite>,
}

impl Favorites {
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
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|f| f.same_target(fav.kind, &fav.target))
        {
            existing.label = fav.label;
            false
        } else {
            self.items.push(fav);
            true
        }
    }

    /// Remove the favorite with this `(kind, target)`. Returns `true` if removed.
    pub fn remove(&mut self, kind: FavoriteKind, target: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|f| !f.same_target(kind, target));
        self.items.len() != before
    }

    /// Toggle membership: remove if present, else append. Returns the new
    /// membership state (`true` = now a favorite).
    pub fn toggle(&mut self, fav: Favorite) -> bool {
        if self.remove(fav.kind, &fav.target) {
            false
        } else {
            self.items.push(fav);
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
        let item = self.items.remove(from);
        self.items.insert(to, item);
    }
}

#[cfg(test)]
#[path = "favorites_tests.rs"]
mod tests;
