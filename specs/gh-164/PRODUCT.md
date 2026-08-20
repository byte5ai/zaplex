# GH-164: Folder history and multi-account launch

## Problem

The Spawn Card remembers neither the project folders used on a host nor more
than one account selection. Repeated work requires navigating to the same folder
again, while comparison or parallel work must be launched one account at a
time. A naive batch loop can duplicate successful launches when one account
fails and the user retries.

GitHub issue: `byte5ai/zaplex#164` (parent: `#160`).

## User experience

The directory row becomes a compact history navigator without adding noise to
the default state.

- Back/forward icons move through the selected host's bounded folder history.
- Opening the history reveals a searchable list ordered by most recent
  successful launch. The path remains the row's primary content.
- Local and every remote host have separate histories. Switching host changes
  both the visible history and the actual launch directory.
- Missing local paths and paths rejected by a connected remote daemon remain
  visible with a muted stale indicator. Confirmation always revalidates the
  selected directory; a stale or unverifiable path cannot launch.

The account row keeps the current automatic/single-account flow and adds
multi-selection plus **all suitable accounts**. Selecting more than one account
shows a stable preview containing count, provider, host, directory, and every
account. Confirmation starts one independent launch per account.

If some launches fail, the card remains open with one concise result per failed
account. **Retry failed** reuses the same launch plan, skips successful targets,
and retries only failures. Successful launches are never duplicated.

## Invariants

- History is written only after at least one launch for that path succeeds.
- Histories are keyed by `local` or the stable SSH registry node id, never by a
  host label.
- Each history is bounded and deduplicated by normalized path. Updating an entry
  moves it to the front.
- A path chosen via history, search, picker, or manual input produces the exact
  same launch path and preview.
- Remote paths are POSIX absolute paths and are validated on their own host;
  local filesystem facts never validate a remote path.
- Batch targets use provider/account/host/directory stable identities. Reordering
  account discovery does not change or duplicate a plan.
- A retry never launches a target already recorded successful for that plan.

## Out of scope

- Synchronizing local filesystem paths between devices.
- Guessing that similarly named hosts share a filesystem.
- Hiding stale history automatically.
- Treating multiple accounts as one shared conversation; each receives a fresh
  provider session and launch intent.

## Success criteria

- A successfully launched folder appears on the next opening for that host.
- Local and remote host histories never mix.
- Back/forward and search update the real launch directory and preview.
- Batch launch starts exactly once per selected account.
- Partial failures are account-specific and idempotently retryable.
- Tests cover MRU ordering/limit, stale paths, host separation, navigation,
  search, preview, stable target identity, and retry idempotency.

## Design input

No separate Figma file exists. The change extends the approved Spawn Card and
Cockpit visual language: existing button themes, icon-only repeated actions,
stable row geometry, and no content revealed on hover.
