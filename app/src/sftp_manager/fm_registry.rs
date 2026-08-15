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

use warpui::{Entity, EntityId, SingletonEntity};

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
    /// The pane group that currently owns this pane. Panes in the source's
    /// group are visible beside it; panes in other groups remain selectable
    /// targets but must not be chosen implicitly.
    pub pane_group_id: Option<EntityId>,
}

/// Candidate destinations for an F5/F6 operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferTargets {
    /// The sole other pane visible beside the source, if there is exactly one.
    pub default: Option<FmPaneDescriptor>,
    /// Every other open pane, including panes in inactive tabs.
    pub selectable: Vec<FmPaneDescriptor>,
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

    /// Resolve the MC default destination without hiding any valid target.
    /// Exactly one other pane in the source's pane group is the default. If
    /// there are zero or multiple visible peers, the caller must present the
    /// complete `selectable` list, including panes in inactive tabs.
    pub fn transfer_targets(&self, self_id: u64) -> TransferTargets {
        let selectable = self.others(self_id);
        let source_group = self
            .panes
            .iter()
            .find(|pane| pane.id == self_id)
            .and_then(|pane| pane.pane_group_id);

        let default = match source_group {
            Some(source_group) => {
                let mut visible = selectable
                    .iter()
                    .filter(|pane| pane.pane_group_id == Some(source_group));
                let only = visible.next().cloned();
                if visible.next().is_none() {
                    only
                } else {
                    None
                }
            }
            None => None,
        };

        TransferTargets {
            default,
            selectable,
        }
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
#[path = "fm_registry_tests.rs"]
mod tests;
