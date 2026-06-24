use std::collections::{HashMap, HashSet};

use bimap::BiMap;
use warp_editor::content::buffer::Buffer;
use warp_util::file::FileId;
use warpui::{ModelContext, ModelHandle, SingletonEntity as _};

use super::server_model::{ConnectionId, ServerModel};
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::remote_server::protocol::RequestId;

/// Distinguishes the type of pending buffer request so the event
/// subscription can send the correct response message.
#[derive(Clone, Copy, Debug)]
pub enum PendingBufferRequestKind {
    OpenBuffer,
    SaveBuffer,
    ResolveConflict,
}

/// Bridges the ServerModel's per-connection state with the GlobalBufferModel's
/// tracked buffers. Manages:
/// - Wire path → FileId mappings for open server-local buffers
/// - Per-buffer connection sets (which connections have each buffer open)
/// - Pending async requests (OpenBuffer, SaveBuffer, ResolveConflict) awaiting events
pub struct ServerBufferTracker {
    /// Bidirectional mapping wire path ↔ FileId, O(1) lookup on both sides.
    /// `path_for_file_id` is called on every `ServerLocalBufferUpdated`, so using
    /// BiMap avoids linear scans.
    open_buffers: BiMap<String, FileId>,
    /// Holds a **strong reference** `ModelHandle<Buffer>` for each open server-local buffer.
    ///
    /// `GlobalBufferModel` internally only stores `WeakModelHandle`; the client keeps the
    /// buffer alive via the editor view's strong reference. But the daemon has no view —
    /// if we don't hold the strong reference here, after `handle_open_buffer` returns, the
    /// buffer's refcount drops to zero and WarpUI's `flush_effects` reclaims it, causing
    /// later when `FileModel` completes async loading, the weak handle is already invalid
    /// (log message "Cannot populate buffer with content due to deallocated model handle").
    /// The buffer is dropped when closed (no more connections).
    buffer_handles: HashMap<FileId, ModelHandle<Buffer>>,
    /// Tracks which connections have each buffer open.
    /// File-watcher pushes go to all connections in the set.
    buffer_connections: HashMap<FileId, HashSet<ConnectionId>>,
    /// Tracks in-flight OpenBuffer / SaveBuffer / ResolveConflict requests so
    /// `GlobalBufferModelEvent`s can be correlated back to the originating
    /// request and connection. Uses a `Vec` to support concurrent requests
    /// for the same buffer from different connections.
    pending_requests: HashMap<FileId, Vec<(RequestId, ConnectionId, PendingBufferRequestKind)>>,
}

impl ServerBufferTracker {
    pub fn new() -> Self {
        Self {
            open_buffers: BiMap::new(),
            buffer_handles: HashMap::new(),
            buffer_connections: HashMap::new(),
            pending_requests: HashMap::new(),
        }
    }

    // ── Path ↔ FileId mapping ─────────────────────────────────────

    /// Register a wire path → FileId mapping and hold a strong reference to the buffer
    /// to keep it alive on the daemon side (see `buffer_handles` field description).
    pub fn track_open_buffer(
        &mut self,
        path: String,
        file_id: FileId,
        buffer: ModelHandle<Buffer>,
    ) {
        self.open_buffers.insert(path, file_id);
        self.buffer_handles.insert(file_id, buffer);
    }

    /// Look up a FileId by its wire path. O(1).
    pub fn file_id_for_path(&self, path: &str) -> Option<FileId> {
        self.open_buffers.get_by_left(path).copied()
    }

    /// Look up the wire path for a given FileId. O(1) via BiMap.
    /// Returns owned `String` rather than `&str`, allowing the caller to hold the result
    /// while borrowing other parts of `&mut self` (typical use: in an event handler, get
    /// the path then call `send_server_message(...)` afterward). Push frequency is low,
    /// so the clone overhead is negligible.
    pub fn path_for_file_id(&self, file_id: FileId) -> Option<String> {
        self.open_buffers.get_by_right(&file_id).cloned()
    }

    // ── Connection tracking ───────────────────────────────────────

    /// Add a connection to a buffer's subscriber set.
    pub fn add_connection(&mut self, file_id: FileId, conn_id: ConnectionId) {
        self.buffer_connections
            .entry(file_id)
            .or_default()
            .insert(conn_id);
    }

    /// Returns the set of connections subscribed to a buffer.
    pub fn connections_for_buffer(&self, file_id: &FileId) -> Option<&HashSet<ConnectionId>> {
        self.buffer_connections.get(file_id)
    }

    /// Remove a connection from all buffer subscription sets.
    /// Returns the list of FileIds that have no remaining connections
    /// (orphaned buffers that should be deallocated).
    pub fn remove_connection(
        &mut self,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<ServerModel>,
    ) -> Vec<FileId> {
        // Discard all pending requests from this connection, avoiding stale entries after disconnect.
        for entries in self.pending_requests.values_mut() {
            entries.retain(|(_, pending_conn_id, _)| *pending_conn_id != conn_id);
        }
        self.pending_requests
            .retain(|_, entries| !entries.is_empty());

        let orphaned: Vec<FileId> = self
            .buffer_connections
            .iter_mut()
            .filter_map(|(file_id, conns)| {
                conns.remove(&conn_id);
                if conns.is_empty() {
                    Some(*file_id)
                } else {
                    None
                }
            })
            .collect();

        for &file_id in &orphaned {
            self.buffer_connections.remove(&file_id);
            self.open_buffers.remove_by_right(&file_id);
            self.pending_requests.remove(&file_id);
            // Release strong reference, allowing the buffer to be reclaimed.
            self.buffer_handles.remove(&file_id);
            GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| gbm.remove(file_id, ctx));
        }

        orphaned
    }

    /// Remove a single connection from a buffer's subscriber set.
    /// If no connections remain, deallocates the buffer entirely.
    pub fn close_buffer(
        &mut self,
        path: &str,
        conn_id: ConnectionId,
        ctx: &mut ModelContext<ServerModel>,
    ) {
        let Some(&file_id) = self.open_buffers.get_by_left(path) else {
            return;
        };

        if let Some(conns) = self.buffer_connections.get_mut(&file_id) {
            conns.remove(&conn_id);
            if !conns.is_empty() {
                return; // Other connections still using this buffer.
            }
        }

        // No connections remain — deallocate.
        self.buffer_connections.remove(&file_id);
        self.open_buffers.remove_by_left(path);
        self.pending_requests.remove(&file_id);
        // Release strong reference, allowing the buffer to be reclaimed.
        self.buffer_handles.remove(&file_id);
        GlobalBufferModel::handle(ctx).update(ctx, |gbm, ctx| gbm.remove(file_id, ctx));
    }

    // ── Pending request tracking ──────────────────────────────────

    /// Stash a pending async request for later correlation with an event.
    pub fn insert_pending(
        &mut self,
        file_id: FileId,
        request_id: RequestId,
        conn_id: ConnectionId,
        kind: PendingBufferRequestKind,
    ) {
        self.pending_requests
            .entry(file_id)
            .or_default()
            .push((request_id, conn_id, kind));
    }

    /// Retrieve and remove pending requests that match `kind` for the given
    /// FileId. Other pending requests for the same FileId are left in place.
    pub fn take_pending_by_kind(
        &mut self,
        file_id: &FileId,
        kind: PendingBufferRequestKind,
    ) -> Vec<(RequestId, ConnectionId)> {
        let Some(entries) = self.pending_requests.get_mut(file_id) else {
            return Vec::new();
        };
        let mut matched = Vec::new();
        entries.retain(|(req, conn, k)| {
            if std::mem::discriminant(k) == std::mem::discriminant(&kind) {
                matched.push((req.clone(), conn.to_owned()));
                false // remove from the vec
            } else {
                true // keep
            }
        });
        if entries.is_empty() {
            self.pending_requests.remove(file_id);
        }
        matched
    }
}
