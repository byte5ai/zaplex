# Agent transcript viewing — technical design

## Context

`app/src/cockpit/pane.rs` currently has three separate actions: Claude delegates to `WorkspaceView::view_transcript`, local Codex loads a one-shot projection in the Cockpit pane, and remote transcripts perform a one-shot `ReadAgentTranscript` request. `app/src/workspace/view.rs` keeps local Claude temp-file watchers in a strong path map and refreshes them on Cockpit reconciliation; it cannot detect document closure or stop when a session becomes dormant. The remote RPC already supports `known_revision` and `NotModified`, and its bounded, pathless reader lives in `app/src/remote_server/transcript_rpc.rs`. The client method is `RemoteServerClient::read_agent_transcript` in `crates/remote_server/src/client/mod.rs`.

The generated document safety boundary is `CodeView::new_generated_read_only` in `app/src/code/view.rs`. It creates a selectable in-memory buffer and already prevents persistence and pane merging.

## Proposed changes

1. Add a provider-neutral transcript target and document projection module under `app/src/cockpit/`. Stable targets contain provider plus exact local account root/session or remote host/account/session identities; display labels are never routing keys.

2. Replace the three Cockpit action paths with one workspace action. The row capability calculation remains in `pane.rs`, but the workspace owns initial loading, generated-document creation, watch registration, and refresh lifetime.

3. Extend `CodeView` with a narrow accessor that returns a weak handle to the generated read-only buffer. The workspace stores only this weak handle, so closing the pane is observable and does not retain the document.

4. Store transcript watches in `WorkspaceView` keyed by the full stable target. Each entry tracks source revision, request generation, in-flight state, and weak buffer. Cockpit reconcile events trigger immediate route checks, while one scoped two-second timer follows only as long as at least one live transcript document remains open.

5. Before each refresh, re-resolve the target against current Cockpit inventory. Dormant or missing local sessions and disconnected/removed/capability-incompatible remote routes stop watching. Remote requests resolve the daemon by exact host id on every attempt.

6. Local Claude and Codex loaders return the shared `TranscriptDocument` plus a content revision derived from the checked source identity. Reads happen on the background executor and reuse the existing bounded provider parsers. Remote responses reuse one strict projection function for both initial read and refresh.

7. A refresh completion applies only when its stable key and request generation still match and the weak buffer upgrades. `NotModified` only clears in-flight state. Loaded or explicit source-state documents replace the buffer atomically; invalid envelopes stop fail-closed.

8. Keep the existing daemon wire schema. `known_revision` and `NotModified` already provide the required conditional polling contract; no path or raw-content protocol is added.

## Testing and validation

- Product 1–3: capability/route tests in `app/src/cockpit/pane_tests.rs` and target identity tests cover both providers, local/remote, stale hosts, and missing capabilities.
- Product 4–8 and 14: workspace watch-state unit tests cover initial registration, unchanged revision, one in-flight request, stale generation, weak-buffer closure, live-to-dormant transition, and duplicate display labels.
- Product 9–13: existing local Codex and daemon transcript fixtures remain, with added local Claude bounded/symlink/replacement cases and remote revision/envelope tests.
- Product 15: daemon concurrency-permit tests remain mandatory; local reads are verified to use the background task boundary.
- Product 16: generated-document tests in `app/src/code/view_tests.rs` and `editor_management_tests.rs` remain green, plus a weak-buffer lifecycle regression test.
- GitHub Actions runs focused `zaplex_cockpit`, remote-server client/server, Cockpit pane, workspace, and generated-document test suites. The Cockpit parity workflow records the result in the final audit artifact.

## Risks and mitigations

- **Polling retains panes:** store only weak buffer handles and remove watches immediately when upgrade fails.
- **Late response retargets a reopened document:** compare both stable target and monotonic request generation before applying.
- **Remote disconnect is mistaken for an empty transcript:** map transport/capability loss to an explicit unavailable state and never fabricate an empty success.
- **UI churn on unchanged content:** use daemon revisions and local checked-source revisions; `NotModified` performs no buffer write.
