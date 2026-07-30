//! Registry of live file-manager panes, so one pane can copy/move into another
//! (the MC F5/F6 verbs). Holds **plain data only** — never view handles — so
//! there is no cross-view borrow or handle-lifecycle risk: each browser pushes
//! a descriptor of itself when its directory changes and removes it on close.
//!
//! The registry is a singleton ([`SingletonEntity`]); it is registered at app
//! startup and in the file-manager test harnesses.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use warpui::{Entity, SingletonEntity};

use super::sftp_backend::SftpBackend;

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

/// How a copy/move between two panes must be carried out, decided purely from
/// the two panes' filesystem namespaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    /// Same filesystem — a direct backend copy / rename, no byte transfer.
    DirectSameFs,
    /// Local source → remote target: upload through the *target's* session.
    Upload,
    /// Remote source → local target: download through the *source's* session.
    Download,
    /// Two different remote hosts: round-trip via the local machine.
    RemoteToRemote,
}

/// Decide how to move bytes from a `source` pane into a `target` pane.
pub fn plan_transfer(source: &FsNamespace, target: &FsNamespace) -> TransferKind {
    match (source, target) {
        (FsNamespace::Local, FsNamespace::Local) => TransferKind::DirectSameFs,
        (FsNamespace::Remote(a), FsNamespace::Remote(b)) if a == b => TransferKind::DirectSameFs,
        (FsNamespace::Local, FsNamespace::Remote(_)) => TransferKind::Upload,
        (FsNamespace::Remote(_), FsNamespace::Local) => TransferKind::Download,
        (FsNamespace::Remote(_), FsNamespace::Remote(_)) => TransferKind::RemoteToRemote,
    }
}

/// The set of currently-open file-manager panes.
///
/// `panes` is plain data (for target discovery + the picker); `backends` is a
/// parallel map of each pane's backend handle, needed so a local pane can
/// upload through a remote pane's session (cross-connection transfers). Both
/// are keyed by pane id and cleared together on close.
#[derive(Default)]
pub struct FileManagerRegistry {
    panes: Vec<FmPaneDescriptor>,
    backends: HashMap<u64, Arc<dyn SftpBackend>>,
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

    /// Record (or replace) a pane's backend handle, so other panes can transfer
    /// through its connection.
    pub fn set_backend(&mut self, id: u64, backend: Arc<dyn SftpBackend>) {
        self.backends.insert(id, backend);
    }

    /// The backend handle for a pane, if it is still open.
    pub fn backend_for(&self, id: u64) -> Option<Arc<dyn SftpBackend>> {
        self.backends.get(&id).cloned()
    }

    /// A live backend for *any* open pane browsing `fs`. Lets a non-file-manager
    /// caller (e.g. the workspace opening a remote file for editing over classic
    /// SSH) borrow an already-established SFTP connection for that host instead
    /// of opening a second one. Returns `None` if no open pane browses `fs` yet
    /// has a backend registered.
    pub fn backend_for_namespace(&self, fs: &FsNamespace) -> Option<Arc<dyn SftpBackend>> {
        self.panes
            .iter()
            .filter(|p| &p.fs == fs)
            .find_map(|p| self.backends.get(&p.id).cloned())
    }

    /// Remove a pane (on close). A no-op if it was never registered. Drops the
    /// pane's backend handle too (releasing its session once nothing else holds it).
    pub fn remove(&mut self, id: u64) {
        self.panes.retain(|p| p.id != id);
        self.backends.remove(&id);
    }

    /// Every other pane that shares `fs` — the candidate copy/move targets for
    /// the pane identified by `self_id`.
    pub fn others_same_fs(&self, self_id: u64, fs: &FsNamespace) -> Vec<FmPaneDescriptor> {
        let mut panes = self
            .panes
            .iter()
            .filter(|p| p.id != self_id && &p.fs == fs)
            .cloned()
            .collect::<Vec<_>>();
        panes.sort_unstable_by_key(|pane| pane.id);
        panes
    }

    /// Every other pane, regardless of filesystem — copy/move candidates
    /// including cross-connection ones (routed via [`plan_transfer`]).
    pub fn others(&self, self_id: u64) -> Vec<FmPaneDescriptor> {
        let mut panes = self
            .panes
            .iter()
            .filter(|p| p.id != self_id)
            .cloned()
            .collect::<Vec<_>>();
        panes.sort_unstable_by_key(|pane| pane.id);
        panes
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

        // `others` ignores the namespace: pane 1 sees 2, 3 and 4.
        let all = reg.others(1);
        assert_eq!(all.len(), 3);
        assert!(all.iter().all(|p| p.id != 1));
    }

    #[test]
    fn backend_handle_is_stored_and_dropped_with_the_pane() {
        use super::super::sftp_backend::InMemorySftpBackend;
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(1, FsNamespace::Local, "/a"));
        assert!(reg.backend_for(1).is_none());

        let backend: Arc<dyn SftpBackend> =
            Arc::new(InMemorySftpBackend::new(PathBuf::from("/")));
        reg.set_backend(1, backend);
        assert!(reg.backend_for(1).is_some());

        // Removing the pane drops its backend handle.
        reg.remove(1);
        assert!(reg.backend_for(1).is_none());
    }

    #[test]
    fn backend_for_namespace_finds_a_live_backend_for_the_host() {
        use super::super::sftp_backend::InMemorySftpBackend;
        let mut reg = FileManagerRegistry::new();
        let host = FsNamespace::Remote("h1".into());
        // Two panes on the same host; only the second has a backend registered.
        reg.upsert(desc(1, host.clone(), "/a"));
        reg.upsert(desc(2, host.clone(), "/b"));
        reg.upsert(desc(3, FsNamespace::Local, "/c"));
        assert!(reg.backend_for_namespace(&host).is_none());

        let backend: Arc<dyn SftpBackend> =
            Arc::new(InMemorySftpBackend::new(PathBuf::from("/")));
        reg.set_backend(2, backend);
        // A pane on the host now has a backend → resolves.
        assert!(reg.backend_for_namespace(&host).is_some());
        // A different host / the local namespace does not.
        assert!(reg
            .backend_for_namespace(&FsNamespace::Remote("other".into()))
            .is_none());
        assert!(reg.backend_for_namespace(&FsNamespace::Local).is_none());

        // Dropping the only backed pane makes it unresolvable again.
        reg.remove(2);
        assert!(reg.backend_for_namespace(&host).is_none());
    }

    #[test]
    fn plan_transfer_covers_every_direction() {
        let local = FsNamespace::Local;
        let host_a = FsNamespace::Remote("a".into());
        let host_a2 = FsNamespace::Remote("a".into());
        let host_b = FsNamespace::Remote("b".into());

        assert_eq!(plan_transfer(&local, &local), TransferKind::DirectSameFs);
        assert_eq!(plan_transfer(&host_a, &host_a2), TransferKind::DirectSameFs);
        assert_eq!(plan_transfer(&local, &host_a), TransferKind::Upload);
        assert_eq!(plan_transfer(&host_a, &local), TransferKind::Download);
        assert_eq!(plan_transfer(&host_a, &host_b), TransferKind::RemoteToRemote);
    }

    #[test]
    fn one_other_pane_is_default_target() {
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(10, FsNamespace::Local, "/source"));
        reg.upsert(desc(20, FsNamespace::Remote("host".into()), "/target"));

        let targets = reg.others(10);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, 20);
    }

    #[test]
    fn hidden_panes_are_targets() {
        let mut reg = FileManagerRegistry::new();
        reg.upsert(desc(10, FsNamespace::Local, "/source"));
        // Simulate panes registered from inactive tabs in an arbitrary
        // activation order. Target discovery must not inherit that order.
        reg.upsert(desc(30, FsNamespace::Remote("hidden-b".into()), "/b"));
        reg.upsert(desc(20, FsNamespace::Remote("hidden-a".into()), "/a"));

        let target_ids = reg
            .others(10)
            .into_iter()
            .map(|pane| pane.id)
            .collect::<Vec<_>>();
        assert_eq!(target_ids, vec![20, 30]);
    }
}
