//! Registry of live file-manager panes, so one pane can copy/move into another
//! (the MC F5/F6 verbs). Holds **plain data only** — never view handles — so
//! there is no cross-view borrow or handle-lifecycle risk: each browser pushes
//! a descriptor of itself when its directory changes and removes it on close.
//!
//! The registry is a singleton ([`SingletonEntity`]); it is registered at app
//! startup and in the file-manager test harnesses.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use warpui::{Entity, SingletonEntity};

/// Process-unique id for a file-manager pane, handed out at construction.
static NEXT_FM_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a fresh pane id.
pub fn next_fm_id() -> u64 {
    NEXT_FM_ID.fetch_add(1, Ordering::Relaxed)
}

/// Which filesystem namespace a pane browses. Copy/move between two panes is
/// only a same-namespace operation for now (local↔remote transfers are a later
/// increment); `Remote` carries the SSH node id so two panes on the *same* host
/// match but two different hosts do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsNamespace {
    Local,
    Remote(String),
}

/// A live snapshot of one file-manager pane, as seen by the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FmPaneDescriptor {
    /// Stable, process-unique id of the pane.
    pub id: u64,
    /// Human label for the destination picker, e.g. `local:/home/u` or
    /// `host:/var/www` (kept live with the pane's current directory).
    pub label: String,
    /// The filesystem this pane browses (for same-namespace matching).
    pub fs: FsNamespace,
    /// The pane's current directory — the copy/move destination.
    pub current_path: PathBuf,
}

/// The set of currently-open file-manager panes.
#[derive(Default)]
pub struct FileManagerRegistry {
    panes: Vec<FmPaneDescriptor>,
}

impl FileManagerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update the descriptor for a pane (keyed by `id`).
    pub fn upsert(&mut self, descriptor: FmPaneDescriptor) {
        if let Some(existing) = self.panes.iter_mut().find(|p| p.id == descriptor.id) {
            *existing = descriptor;
        } else {
            self.panes.push(descriptor);
        }
    }

    /// Remove a pane (on close). A no-op if it was never registered.
    pub fn remove(&mut self, id: u64) {
        self.panes.retain(|p| p.id != id);
    }

    /// Every other pane that shares `fs` — the candidate copy/move targets for
    /// the pane identified by `self_id`.
    pub fn others_same_fs(&self, self_id: u64, fs: &FsNamespace) -> Vec<FmPaneDescriptor> {
        self.panes
            .iter()
            .filter(|p| p.id != self_id && &p.fs == fs)
            .cloned()
            .collect()
    }

    /// All registered panes (for tests/diagnostics).
    pub fn panes(&self) -> &[FmPaneDescriptor] {
        &self.panes
    }
}

impl Entity for FileManagerRegistry {
    type Event = ();
}

impl SingletonEntity for FileManagerRegistry {}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(id: u64, fs: FsNamespace, path: &str) -> FmPaneDescriptor {
        FmPaneDescriptor {
            id,
            label: format!("pane{id}"),
            fs,
            current_path: PathBuf::from(path),
        }
    }

    #[test]
    fn upsert_inserts_then_updates_in_place() {
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(1, FsNamespace::Local, "/a"));
        reg.upsert(desc(2, FsNamespace::Local, "/b"));
        assert_eq!(reg.panes().len(), 2);

        // Same id → replace, not append.
        reg.upsert(desc(1, FsNamespace::Local, "/a2"));
        assert_eq!(reg.panes().len(), 2);
        assert_eq!(
            reg.panes().iter().find(|p| p.id == 1).unwrap().current_path,
            PathBuf::from("/a2")
        );
    }

    #[test]
    fn remove_is_idempotent() {
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(1, FsNamespace::Local, "/a"));
        reg.remove(1);
        reg.remove(1); // no panic, no-op
        assert!(reg.panes().is_empty());
    }

    #[test]
    fn others_same_fs_excludes_self_and_other_namespaces() {
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(1, FsNamespace::Local, "/a"));
        reg.upsert(desc(2, FsNamespace::Local, "/b"));
        reg.upsert(desc(3, FsNamespace::Remote("host1".into()), "/c"));
        reg.upsert(desc(4, FsNamespace::Remote("host2".into()), "/d"));

        // From pane 1 (local): only pane 2 is a valid local target.
        let targets = reg.others_same_fs(1, &FsNamespace::Local);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, 2);

        // From pane 3 (host1): no other host1 pane exists.
        assert!(reg
            .others_same_fs(3, &FsNamespace::Remote("host1".into()))
            .is_empty());
    }
}
