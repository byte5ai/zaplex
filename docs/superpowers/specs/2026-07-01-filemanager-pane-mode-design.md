# File Manager — Pane-Mode Design (the "MC, but better")

> zaplex's integrated file manager is **not** a hardwired dual-pane MC widget and
> **not** a sidebar tool. It is a **view mode of a pane**: any terminal pane (local
> or remote) can toggle terminal ⇄ file-manager, keeping the same connection
> context. The classic MC two-column layout emerges when the user puts two panes
> into file-manager mode — it is a *superset* of MC (N file managers, any split/tab
> arrangement, even across tabs), not a copy of it.
>
> Grounded in the existing pane system (`app/src/pane_group/pane/`, trait
> `PaneContent` at `pane/mod.rs:590`), the existing SFTP backend abstraction
> (`app/src/sftp_manager/sftp_backend.rs`: `LiveSftpBackend` over `zap_sftp::Sftp`,
> `InMemorySftpBackend` over the local FS), the existing `sftp_pane.rs` /
> `file_pane.rs` / `terminal_pane.rs`, and the sidebar `server_file_browser.rs`
> (which this design retires as the primary entry point). See
> [[filemanager-pane-mode-design]], [[no-quick-wins-sustainable-only]],
> [[remote-session-arch-decision]].

## 1. Goal & Non-goals

**Goal.** Make "file manager" a first-class **pane mode** so the user can:
1. toggle any terminal pane to a file-manager view (and back) **in place**, keeping
   the pane's connection context (local cwd, or the host's SSH/daemon session);
2. arrange any number of file-manager panes in any split/tab layout (1 beside a
   shell, 2 side-by-side = MC, 3+ anywhere, across tabs);
3. copy/move files between **any** active file-manager panes — including ones in
   hidden tabs — with a clear picture of source and destination paths;
4. operate it fast: discoverable for beginners (icons/menus), keyboard-driven for
   pros (hotkeys, MC-style function keys).

**Non-goals (explicitly out / deferred):**
- Not a separate MC application, not a modal overlay, not a sidebar panel. The
  sidebar `ServerFileBrowser` tab is retired as the primary FM entry point
  (may remain as a quick "jump to host root" shortcut, TBD).
- No new remote transport: remote FM reuses the existing SFTP over the host's SSH
  connection. (Daemon-native file ops are a possible later optimization, not P1.)
- No cloud/object-store anything (self-contained; see [[self-contained-audit-findings]]).
- Archive browsing (zip/tar as a directory), FTP/S3/other protocols — later, if ever.

## 2. Why this shape (mission fit)

zaplex is a premium remote-dev terminal for people running many local + remote
sessions ([[mission-and-reference-sources]]). A file manager that is *a mode of the
pane you already have* means: split your shell, toggle, and you are browsing exactly
where you were — including on the remote host the shell is connected to. The
cd-continuity and the "any layout" freedom are things MC structurally cannot do.
It also reuses the pane system (splits, tabs, promotion, drag-drop) instead of
adding a bespoke widget, which keeps the UX consistent and the surface area small.

## 3. Core model

### 3.1 Connection context (the linchpin)

Introduce an explicit **`PaneContext`** shared by terminal and file-manager panes:

```
enum PaneContext {
    Local,                      // the machine zaplex runs on; has a cwd
    Remote { node_id: String }, // an SSH host node; browsed over that host's SFTP
}
```

- A terminal pane already *has* such a context implicitly (a local PTY, or a
  daemon/SSH session to a host — see `terminal_pane.rs`, which already forwards
  "open remote file" events to the workspace). We make it explicit and attach the
  **current directory** to it (from the shell's cwd where available; otherwise last
  browsed path or home).
- Toggling terminal ⇄ FM **preserves `PaneContext` and the current directory.** This
  is the killer feature: `split → toggle` opens the FM rooted at the shell's cwd.

### 3.2 Pane mode toggle

Panes are trait objects (`PaneContent` + a `BackingView`), so "mode" is realized by
swapping the pane's content implementation **within the same pane slot** (same
`PaneId`, same position in the `PanesLayout`), preserving focus and neighbors:

- `TerminalPane` ⇄ `FileManagerPane`, both carrying the same `PaneContext`.
- Toggling is reversible and cheap; the terminal session is **not** torn down when
  showing the FM (it is suspended/hidden), so toggling back is instant and the shell
  keeps running. (Trade-off vs. memory noted in §10.)
- Also allow **opening** an FM pane directly (new split / new tab) with a chosen
  context, without starting from a terminal.

### 3.3 The `FileManagerPane`

A `FileManagerPane` = one `FsBackend` (see §4) + view state:
- current path (+ history: back/forward/up), selection set, sort, column config
  (name/size/mtime/perms/owner), quick-filter/incremental-search, show-hidden.
- editable **path bar / breadcrumb**; **connection identity** shown in the header
  (local hostname or remote host) so the user always knows *which machine*.
- registered in the **FM registry** (§5) on attach, deregistered on close.

## 4. Filesystem backend abstraction

Reuse and generalize the existing SFTP backend split. Today `sftp_backend.rs` has a
backend trait with `LiveSftpBackend` (real `zap_sftp::Sftp`) and `InMemorySftpBackend`
(local FS, used for tests). Promote this to a first-class **`FsBackend`** used by the
FM pane:

```
trait FsBackend {                         // async, cancelable
    async fn list_dir(&self, path) -> Vec<DirEntry>;
    async fn stat/mkdir/rename/remove/read_link/set_perms(...);
    fn open_read(&self, path) -> ByteStream;    // for transfers
    fn open_write(&self, path) -> ByteSink;
    fn capabilities(&self) -> FsCaps;           // perms? symlinks? rename-across-dir?
}
```

- `LocalFsBackend` — the local machine (generalize `InMemorySftpBackend` into a real
  local backend).
- `SftpFsBackend` — wraps the host's existing SFTP connection (from
  `LiveSftpBackend`); **no new connection** — piggybacks the SSH session zaplex
  already holds for that host.
- Backends are keyed by `PaneContext`, so two FM panes on the same host share one
  SFTP channel.

## 5. Copy / move target model (the "better than MC" part)

### 5.1 FM registry

A singleton **`FileManagerRegistry`** model tracks every open `FileManagerPane`:
`{ pane_id, PaneContext, current_path (live), is_visible, tab_id }`, emitting change
events. This is what makes cross-tab targeting possible.

### 5.2 The copy/move dialog

When the user copies/moves the current selection, the dialog offers **all *other*
active FM panes** as destinations — including ones in hidden tabs — each shown as:

```
  ●  devhost : /srv/www/app/releases        (visible, this tab)
  ○  laptop  : ~/projects/zaplex            (hidden tab "build")
  ○  prod-db : /var/backups                 (hidden tab "ops")
  +  Enter path manually…      ★ Bookmarks…
```

Each row shows the destination's **live current path** — so, exactly like MC's
"other panel", the user sees precisely where files will land.

### 5.3 Default target heuristic (MC speed for the MC case)

- If the current tab has **exactly one other visible FM pane**, it is the
  pre-selected default target → the classic MC flow: **F5 = copy, F6 = move**, one
  keypress, no dialog (dialog still reachable for retargeting).
- Otherwise the dialog opens with the pick-list. So the common side-by-side case is
  as fast as MC, and the general case (many FMs, cross-tab) is still one clear pick.
- Manual path entry + bookmarks always available.

## 6. Transfer engine (the real backend work)

Cross-context transfers are where the substance is; this gets its own module with a
proper progress/queue UI:

- **local ↔ remote**: SFTP up/down over the host's connection.
- **remote-A ↔ remote-B**: relay through local (stream A→local→B); direct
  server-to-server only if same host.
- **local ↔ local / same-remote**: native rename when possible, else copy+delete.
- A **transfer queue** with: per-item + aggregate progress, throughput/ETA, pause /
  cancel, and **conflict resolution** (skip / overwrite / rename / newer-only,
  apply-to-all). Must handle large files (streamed, not buffered), directory
  recursion, symlinks, and permission/owner preservation per `FsCaps`.
- Transfers survive tab switches (they live in the queue model, not the pane) and
  surface in a small global activity indicator.

## 7. UX

**Discoverable (beginners):**
- Pane header control: an icon toggle **terminal ⇄ files**.
- Right-click pane menu: "Switch to File Manager" / "Switch to Terminal".
- Command palette entries; the `+` menu can offer "File Manager" as a pane/tab.

**Fast (pros) — proposed hotkeys (final binding TBD, must not clash):**
- Toggle current pane terminal⇄FM.
- **Toggle *all* panes in the current tab at once** — the "make this an MC" gesture.
- `F5` copy / `F6` move to default target; `F7` mkdir; `F8`/`Del` delete; `F2`
  rename; `Tab` cycle focus between FM panes; `Enter` open; `Backspace` up.
- Optional classic MC function-key legend along the pane bottom (toggleable).

**Visual quality:**
- The **active pane** gets an accent border (uses the new Zaplex-Dark accent
  `#6C82F2`) — source vs. destination must be unambiguous.
- Selection, focus, and the "this is a remote path" state are visually distinct.
- **Drag & drop** between FM panes is the mouse-first path and reuses the exact
  copy/move + conflict logic from §5–6.

## 8. Phasing

- **P1 — FM as a pane mode.** `PaneContext`, terminal⇄FM toggle, `FileManagerPane`
  over `FsBackend` (Local + Sftp), single-pane browse + in-place ops
  (open/rename/mkdir/delete/chmod), path bar, header identity. No cross-pane
  transfer yet. Retire the sidebar browser as the primary entry.
- **P2 — Targeting + transfers.** FM registry, copy/move dialog with cross-tab
  destinations + live paths, default-target heuristic (F5/F6), and the transfer
  engine for **local↔remote** with the progress/conflict queue.
- **P3 — Full power.** remote↔remote relay, drag & drop, "toggle all panes in tab"
  gesture, MC function-key parity/legend, bookmarks.

## 9. Testability

- `FsBackend` is mockable (the in-memory/local backend already exists) → browse
  logic, ops, and the transfer engine are testable **headless** (no GUI, no network),
  matching the project's data-first, unit-tested increment style.
- Registry + default-target heuristic are pure logic → unit tests over synthetic
  pane/tab layouts.
- Conflict resolution + large-file streaming get dedicated tests (fixtures).

## 10. Open questions

1. **Suspended terminal on toggle**: keep the PTY alive+hidden (instant toggle,
   more memory) vs. detach on toggle (cheaper, slower return)? Proposal: keep alive;
   for daemon-backed remote sessions it survives regardless.
2. **Sidebar `ServerFileBrowser`**: retire fully, or keep as a lightweight "open FM
   pane at host root" launcher?
3. **Exact hotkeys** and whether to ship the MC function-key legend on by default.
4. **remote↔remote direct** (SFTP server-to-server) where supported vs. always relay.
5. Do we expose a "sync/mirror two panes" power feature later, or keep scope to
   copy/move?

## Appendix — code anchors

- Pane system / trait: `app/src/pane_group/pane/mod.rs` (`PaneContent` :590,
  `AnyPaneContent` :663), layout `app/src/pane_group/mod.rs` (`PanesLayout` :773,
  `PaneDragDropLocation` :789).
- Existing panes: `pane/terminal_pane.rs` (already forwards remote-file events),
  `pane/sftp_pane.rs`, `pane/file_pane.rs`, `pane/ssh_server_pane.rs`.
- SFTP backend to generalize: `app/src/sftp_manager/sftp_backend.rs`
  (`LiveSftpBackend`, `InMemorySftpBackend`), ops in `sftp_manager/sftp_ops.rs`,
  `zap_sftp::Sftp`.
- Sidebar browser being retired as primary: `app/src/workspace/view/server_file_browser.rs`.
