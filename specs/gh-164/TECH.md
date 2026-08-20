# GH-164: Technical design

## Architecture

Three layers keep persistence and retry policy independent from WarpUI.

1. `app/src/workspace/view/spawn_card_history.rs` owns the versioned,
   per-host MRU store, cursor navigation, search projection, validation state,
   atomic persistence, and corruption protection.
2. `app/src/workspace/view/spawn_card_bulk.rs` owns stable account target ids,
   launch-plan construction, preview data, result accounting, and idempotent
   retry selection.
3. `spawn_card.rs` renders those models and emits validation/launch events.
   `WorkspaceView` performs host validation and launches, then returns structured
   results to the card.

## Folder history

`FolderHistoryHost` is either `Local` or `Remote { node_id }`. The remote key is
the SSH registry node id used by Spawn Card routing. Display names are metadata
only.

The persisted schema contains a version and a map from host key to at most 20
entries. Each entry contains normalized UTF-8 path and last-success timestamp;
validation state is deliberately not persisted because it becomes stale. The
store uses a temporary file, fsync, and atomic replace. Corrupt or unreadable
input is protected from overwrite.

Navigation maintains a per-open cursor over a snapshot of the current host's
history. Choosing a new path truncates forward history in the navigation model
without deleting persisted MRU entries. Search is case-insensitive substring
matching over the path and never changes selection until the user activates a
result.

Local validation uses metadata and requires a directory. Remote validation uses
the connected daemon's `list_directory` request against the exact node's live
daemon. A disconnected host or protocol error is `Unverifiable`, not `Valid`.
The generation attached to requests prevents a late result for another host or
path from enabling Confirm.

## Stable bulk plans

Every account option supplies a stable identity:

- local: provider plus normalized config-directory identity;
- remote: provider plus daemon-issued opaque account id.

A `BulkLaunchPlan` receives one stable operation id when Confirm is first
pressed. Its immutable targets contain account identity, account route, host,
directory, provider, model, effort, and prompt. Target ids are deterministic
within the plan and do not depend on discovery order.

`BulkLaunchLedger` records `Pending`, `Succeeded { launch_id }`, or
`Failed { message }` per target. `targets_for_attempt` returns only Pending and
Failed targets; succeeded targets cannot be returned. Applying the same success
twice is idempotent. Changing any steering selection invalidates the current
plan and creates a new one on the next confirmation.

## Workspace execution

The existing single-launch function gains a structured internal result while
retaining its toast wrapper for legacy callers. For every batch target the
workspace:

1. revalidates the selected path on its exact host;
2. revalidates the account route against current discovery;
3. creates a launch-registry id before opening the process;
4. opens exactly one terminal and attaches the launch id;
5. returns the per-target result to the card.

On complete success the card closes and the history store records the directory.
On partial success the card stays open, records the successful history once,
and renders failed accounts plus Retry failed.

## Verification

Pure tests cover MRU bound/dedup, host separation, navigation/search, corrupt
store protection, stale validation generations, preview formatting, plan
stability under reorder, per-account partial failures, duplicate completion,
and retry selection. Workspace tests cover path/account revalidation and one
launch-registry intent per successful target.

No local Cargo command is run; CI performs `cargo check` and focused tests.
