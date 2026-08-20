# GH-162: Complete session lifecycle controls

## Problem

Zaplex can stop, kill, fork, adopt, inspect, and review agent sessions, but it
cannot yet restart an existing conversation in place, persistently rename it,
or safely remove an orphaned session-registry record. The missing verbs force a
user to leave the Cockpit and reconstruct account, host, model, and conversation
context manually. A naive cleanup is worse than no cleanup because a reused PID
or temporarily unreachable host can make a live session look stale.

GitHub issue: `byte5ai/zaplex#162` (parent: `#160`).

## User experience

The session overflow menu contains only lifecycle verbs that are executable for
that exact row.

- **Restart** is offered for a live, exactly located Claude or Codex pane whose
  process identity can be revalidated and whose provider supports resuming the
  same conversation. It ends the current provider process and queues a routed
  resume in the same pane. The account, host, working directory, conversation,
  model, effort, and known launch flags remain unchanged.
- **Rename** is offered only where the provider or its authoritative session
  registry supports persistent naming. A small dialog previews the current
  identity, rejects empty/control-bearing names and collisions in the same
  provider/account/host namespace, and applies the name only after confirmation.
- **Clean up stale entry** appears only for an explicitly identified orphaned
  registry record. The row remains visible and selectable until the user
  confirms cleanup. Cleanup revalidates the same account route, provider
  session, registry revision, and process identity immediately before mutation.

Capabilities disappear when the exact route is unavailable. A disabled-looking
verb that cannot work is not rendered. Remote rows use daemon and opaque account
identities, never host labels or client-side config paths.

## Invariants

- Restart never forks or creates a new provider conversation.
- Restart never moves to another pane, account, host, or working directory.
- A restart begins only after termination of the selected process has been
  acknowledged; failure leaves the original launch intent available for retry.
- Rename uniqueness is scoped by provider, account, host, and provider session
  namespace. The same display name on another account or host is allowed.
- A rename is not considered successful until the authoritative source accepts
  it; a rescan and app restart must reproduce the new name.
- Cleanup is fail-closed for PID reuse, missing process fingerprints, changed
  registry revisions, disconnected hosts, ambiguous account routes, and probe
  errors. Dormant transcript history is not stale merely because no process is
  running.
- Every partial failure names the affected session and remains retryable without
  repeating a successful mutation.

## Out of scope

- Inventing rename support for providers that expose no persistent naming
  mechanism.
- Deleting transcripts or conversation history.
- Treating an unreachable remote host as evidence of staleness.
- Replacing Stop/Kill guardrails or their confirmation policy.

## Success criteria

- Restart continues the same conversation with the exact original route and
  launch intent in the same pane.
- Rename survives a Cockpit rescan and application restart.
- Cleanup cannot remove a live, PID-reused, ambiguous, or unverifiable entry.
- Local and remote actions address the same provider/account/session key used by
  discovery.
- Focused tests cover restart preservation, rename conflicts and persistence,
  PID reuse, remote exact routing, partial failure, and stale fail-closed paths.

## Design input

No separate Figma mock exists for these menu-level lifecycle verbs. They reuse
the approved Cockpit row-menu and shared modal grammar; hover never changes row
geometry.
