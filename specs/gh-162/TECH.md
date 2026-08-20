# GH-162: Technical design

## Architecture

The implementation is split into pure lifecycle policy and thin UI/transport
adapters.

1. `app/src/cockpit/session_lifecycle.rs` owns stable route keys, capability
   decisions, restart intents, rename validation, stale cleanup proofs, and the
   retry state machine. These types are independent of WarpUI and are tested
   directly.
2. `app/src/cockpit/capabilities.rs` projects the lifecycle policy into the row
   menu. It never re-derives provider support in the renderer.
3. `app/src/cockpit/pane.rs` constructs actions from the selected row's complete
   route. Rename uses a stable modal/input surface; cleanup is explicitly
   confirmed.
4. `app/src/workspace/view.rs` resolves the route against a fresh inventory and
   exact terminal binding before any mutation. Local and remote adapters return
   structured success/failure results to the lifecycle state machine.
5. `app/src/cockpit/launch_registry.rs` supplies the exact bound launch record
   for restart and replaces its terminal-scoped intent before resume. It also
   exposes exact, bounded removal for stale Zaplex launch records.

## Stable identity

`SessionLifecycleRoute` contains:

- provider and provider session id;
- local marker or stable remote daemon host id;
- local config-dir pin or remote opaque account id;
- account email when known;
- process PID plus provider-observed process fingerprint;
- working directory.

Display labels are excluded from equality and routing. Remote config paths are
never transported to the client.

## Restart sequence

1. Resolve the route against the current inventory and require exactly one row.
2. Resolve exactly one existing terminal with the same host/account/session key.
3. Resolve the exact launch record. Unknown launch flags fail closed rather than
   being replaced with defaults.
4. Re-probe the selected process and compare its fingerprint. PID reuse or probe
   uncertainty aborts.
5. Ask the existing local guardrail primitive (or negotiated daemon signal RPC)
   to terminate the exact process.
6. After acknowledged termination, attach a new stable launch intent to the same
   terminal, clear only the old provider-session binding, and queue the
   provider's routed resume command with the preserved model/effort.
7. A hook event binds the same provider conversation id to the new launch intent.

The command builder is provider-owned in `CLIAgent`; it quotes the session id,
scrubs API-key overrides, preserves the account pin, and appends only verified
model/effort flags.

## Rename

Rename adapters implement a small authoritative interface. Claude live sessions
use the provider's native rename command in the exact pane. Providers without a
native command use only a versioned Zaplex session-registry overlay when that
overlay is the declared authoritative source for their Cockpit label. The store
is keyed by the complete lifecycle route (without volatile process data), writes
atomically, protects corrupt input from overwrite, and rejects same-scope name
collisions. Discovery applies the overlay before constructing the fleet tree.

Remote rename is available only when the connected daemon advertises the
matching lifecycle feature; otherwise the capability is absent.

## Stale cleanup

The scanner emits a cleanup candidate only for a provider registry entry whose
process is proven gone and whose transcript remains independently addressable.
The candidate carries an opaque revision derived from provider/session/account,
registry content identity, PID, and process-start marker. It does not expose a
remote path.

Cleanup performs a second scan and rejects:

- a live/unknown process;
- changed PID, start marker, content identity, or account route;
- multiple matching entries;
- an unreachable remote daemon;
- a candidate that has already been replaced.

Successful cleanup removes only the exact orphaned registry record. Transcript
history remains. A repeated cleanup reports `AlreadyApplied` and performs no
second mutation.

## Error and retry model

Every lifecycle attempt has a stable operation id and terminal state:
`Pending`, `Applied`, or `Failed { retryable, message }`. An applied operation is
idempotent. A retry creates no new target identity and cannot cross account or
host boundaries.

## Verification

Dedicated tests cover:

- restart command and launch-record preservation for Claude and Codex;
- same-pane terminal identity and remote route matching;
- rename validation, namespace collisions, atomic persistence, and rescan load;
- stale candidate construction, PID reuse, changed revisions, uncertain probes,
  remote disconnect, and idempotent cleanup;
- capability absence for incomplete routes and unsupported providers.

No local Cargo command is run; CI performs `cargo check` and focused tests.
