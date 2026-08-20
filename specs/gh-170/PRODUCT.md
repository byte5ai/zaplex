# Agent transcript viewing

## Summary

Zaplex opens Claude and Codex conversation history through one safe, provider-neutral viewer for local and supported remote sessions. Live conversations stay current while their document is open; dormant conversations are immutable snapshots and never keep a watcher alive.

## Figma

Figma: none provided. The approved Cockpit hierarchy and interaction decisions in `specs/GH-160/PRODUCT.md` remain the visual source of truth.

## Behavior

1. A transcript action is visible only when Zaplex can resolve the exact provider, account, host, and session through a supported local loader or a currently connected daemon capability.

2. Local Claude and local Codex actions open the same selectable, read-only, non-persistent document surface. The provider is explicit in the document title, and the document contains only normalized user/assistant text, bounded reasoning summaries, compact tool names, model labels, and timestamps supported by the source.

3. A supported remote action sends only opaque provider, account, host, and session identities to the exact connected daemon. It never sends or interprets a remote filesystem path on the client.

4. Opening a live session performs an initial bounded read and then follows source revisions while the document remains open and the same session remains live.

5. An unchanged source revision does not rewrite the document, move selection, or create another pane.

6. At most one refresh for a document is in flight. Late responses from an older refresh generation cannot overwrite newer contents or a document that has been closed and reopened.

7. Closing the document releases its watcher. A document handle must not be retained merely to keep polling alive.

8. When a watched session becomes dormant, Zaplex applies any already accepted latest snapshot and stops following it. Reopening dormant history performs one read and creates no watcher.

9. Local and remote loading, missing, empty, unsupported, malformed, too-large, temporarily unavailable, and disconnected states are distinct and visible. Unknown values are never rendered as an empty successful transcript.

10. A transient read failure is scoped to the affected document and can recover on a later valid revision while the exact live route still exists. A stale, ambiguous, removed, or capability-incompatible route stops fail-closed and is never retargeted by display name.

11. Local source reads enforce the same bounded-file, regular-file, symlink, provider-root, line, turn, and display-size constraints as daemon reads. Replacement between resolution and read fails closed.

12. Remote daemon responses are revalidated for schema version, provider, session identity, status, turn count, field sizes, timestamps, and revision shape before they affect a document.

13. Raw transcript lines, tool arguments/results, encrypted payloads, credentials, provider config paths, and unrelated files never enter the generated document or cross the remote protocol.

14. Multiple open transcript documents use independent stable watch keys. Identical display titles, project names, or account labels cannot merge their state.

15. Local and remote readers remain responsive when a source is large or slow: filesystem work and daemon requests run off the UI thread, concurrency is bounded, and one busy source reports its own unavailable state without blocking other documents.

16. Keyboard selection, copy, theme behavior, generated-document save prevention, restore prevention, and pane-merge prevention remain identical to the existing generated read-only document surface.
