# GH-169 — Executable Cockpit parity audit

## Problem

The Cockpit parity claim currently combines strong source-level evidence with a manual runtime
checklist. The automated report cannot distinguish between a real runtime pass, an invalid evidence
bundle, and a run that never happened. That is too weak for a release decision and makes it easy to
mistake missing evidence for success.

## Product outcome

Zaplex has two explicit, complementary gates:

1. The automated parity gate validates fresh reference revisions, the versioned provider/account/
   host/status matrix, focused Rust suites, and normal/narrow/reduced-motion screenshots.
2. The runtime release gate validates a sanitized evidence bundle captured with the exact Zaplex
   revision on a real two-host topology: the local host plus one connected remote host.

The automated gate remains useful on every relevant pull request. The runtime gate is deliberately
not synthesized in hosted CI because installed authenticated provider CLIs and the user's remote
transport are not available there.

## User-visible contract

- The runtime report has exactly three possible states: `pass`, `fail`, or `not-run`.
- Missing evidence is always `not-run`; it is never treated as a pass.
- Invalid, incomplete, stale, revision-mismatched, or duplicated evidence is `fail`.
- A release-gate invocation succeeds only when both the automated report and the runtime report
  pass for the same Zaplex revision.
- Runtime evidence covers local Claude, local Codex, one remote-host provider session, exact
  reattach/lifecycle behavior, the machine-readable snapshot, and last-connection host removal.
- The two-host topology is exactly one sanitized local label and one sanitized remote label.
- Screenshots are required for local Claude, local Codex, the remote host, and reattach. They must
  be distinct PNG files with plausible dimensions. Normal, narrow, and reduced-motion screenshots
  remain deterministic automated artifacts.
- The evidence bundle accepts no extra files, symlinks, raw host ids, account identities, provider
  tokens, transcript text, or personal filesystem paths.

## Non-goals

- Hosted CI does not log into Claude or Codex accounts.
- The audit does not depend on either reference repository at application runtime.
- A screenshot file check does not replace the human responsibility to inspect and sanitize the
  visible image before attaching it.
- This issue does not change Cockpit production behavior.

## Acceptance criteria

1. Relevant pull requests and manual workflow dispatches run `cargo check`, the focused Cockpit and
   daemon suites, reference synchronization/probes, negative gate self-tests, and UI screenshots.
2. Provider, host, status, missing-test, and runtime-evidence mutations fail deterministically.
3. The assembled report contains the Zaplex revision, both reference branches/SHAs, audit time,
   automated component results, and the explicit runtime state.
4. A documented command validates a real runtime bundle and can require a pass for a release.
5. The manual procedure covers local Claude, local Codex, one remote host, and the combined
   two-host lifecycle without exposing secrets.
6. GH-160 is not considered completely verified until a linked report shows both automated and
   runtime release gates passing for the same revision.
