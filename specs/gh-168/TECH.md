# Managed Remote-Control Fleet — Technical Specification

## Context

GitHub issue #168 extends the native remote session layer into an explicit managed-agent fleet. The updated reference audit uses `claudeplex` commit `8c2041ff68d97463aed7aeb01da0f16b708b8e22`; its useful behaviors are durable Claude Remote Control and PSS-based fleet accounting. Zaplex deliberately uses a stronger boundary: daemon-owned PTYs and generation-checked protocol identities instead of `tmux`, `ps eww`, config-directory transport, or command-line matching.

- `app/src/remote_server/session_host.rs:80-135` owns each daemon PTY, child process, attachment, replay ring, and last-attach timestamp.
- `app/src/remote_server/server_model.rs:3000-3320` validates opaque account routing and opens daemon PTYs.
- `app/src/remote_server/server_model.rs:3590-3780` implements detach, inventory, and detached-session GC. Today GC exempts only sessions with live connections.
- `crates/remote_server/proto/remote_server.proto:790-950` defines open/attach/detach/close/list messages and their generation fields.
- `crates/remote_server/src/client/mod.rs:730-1135` owns typed client calls for the session protocol.
- `crates/zaplex_remote_session/src/types.rs:1-210` owns negotiated capability constants.
- `app/src/workspace/view/spawn_card.rs` and `app/src/workspace/view.rs` route account-scoped remote launches.
- `app/src/cockpit/panel.rs` renders the compact sidebar tree; `app/src/cockpit/pane.rs` owns the full Cockpit details and lifecycle actions.

The issue body is the approved product input. `PRODUCT.md` makes the fail-closed memory and security behavior explicit.

## Proposed changes

### 1. Pure fleet identity and launch policy

Add `app/src/remote_server/managed_fleet.rs` with platform-independent types and validation:

- `ManagedFleetIdentity` contains the daemon host id, opaque account id, project root, provider, PTY session id, and generation. It rejects empty components and never stores an account config path.
- `ManagedLaunchKey` is the idempotency key for host/account/project/provider before a PTY exists.
- `ManagedLaunchKind::InteractiveAgent` supports account-routed Claude and Codex keepalive sessions through their canonical CLI entrypoints. `ManagedLaunchKind::ClaudeRemoteControl` adds Claude's official mobile/share mode; unsupported providers fail explicitly.
- `ClaudeRemoteControlSpec` constructs only the documented `claude remote-control` argv (`--spawn`, `--capacity`, optional `--permission-mode`, optional display name). Its startup delivery escapes every argv item independently through the active shell family and uses the existing retry-safe daemon PTY input; no environment-prefix command is generated.
- `HeadroomPolicy` combines a daemon-configured floor with an optional stricter client request. The daemon floor always wins when larger.
- The daemon reads `ZAPLEX_MANAGED_MIN_AVAILABLE_MB` once at startup; absent means 2048 MiB. Parsing is strict, zero/invalid/overflow values disable managed starts with a configuration error rather than disabling the gate. This value is daemon policy, not launch-command environment.
- `evaluate_headroom` returns an allow decision only for a fresh, measured `MemAvailable` value at or above the effective floor. Unsupported, unavailable, stale, malformed, and below-floor results are typed denials.

### 2. Linux memory measurement with honest degradation

Add `app/src/remote_server/fleet_memory.rs`:

- A small read-only `ProcfsReader` boundary makes parsing deterministic in tests without mocking global `/proc`.
- Linux production collection reads `/proc/meminfo` and `/proc/<pid>/smaps_rollup`. It accepts `kB` only, checks multiplication overflow, and returns `Measurement::Unavailable` rather than `0` on every read/parse/permission/process-race failure.
- PSS aggregation captures the daemon-owned shell PID's Linux start time and kernel process-session id. Each refresh scans only that kernel session, sorts/deduplicates PIDs, revalidates start time after every `smaps_rollup` read, and returns unavailable if any requested process cannot be measured. This avoids PID-reuse races and undercounting a partially readable tree, including re-parented descendants that remain in the PTY's process session.
- Non-Linux production collection returns `Measurement::Unsupported` with platform provenance. It never claims Linux procfs support.
- `FleetMemorySnapshot` includes collection epoch, provenance, host available bytes, optional process PSS, and policy floor; it contains no process environment, command line, or config path.

### 3. Daemon session state and GC

The central integration adds managed metadata to `session_host::Session`:

- `managed: Option<ManagedSessionMetadata>`; `None` preserves legacy behavior.
- Metadata contains `ManagedLaunchKey`, launch kind/config, and the exact child/process-group identity needed for memory aggregation. It does not contain credentials.
- Detached-age GC and ring-cap candidate selection exclude managed sessions. Explicit stop or process EOF still remove them and unbind the agent PTY.
- Daemon idle shutdown must treat any live managed session as live work, so it cannot exit while keeping the PTY is required.
- Recent exit status is bounded and secret-free; no automatic restart loop is introduced.

### 4. Additive wire protocol (central integration owner)

The parent integration owns the shared proto and routing edits to avoid conflicts with issue #170. The exact proposed additive schema is:

1. Capability: `managed-agent-fleet-v1`, advertised only by daemons implementing all lifecycle and memory semantics; clients list it platform-independently but require daemon advertisement.
2. `OpenSession` additions:
   - field `7`: `ManagedLaunch managed_launch` (message presence, absent means ordinary PTY)
   - field `8`: `optional uint64 requested_min_available_bytes`
3. `ManagedLaunch`:
   - field `1`: `uint32 schema_version` (= 1)
   - field `2`: `string launch_id` (stable client idempotency id)
   - field `3`: `string provider`
   - field `4`: `string project_root`
   - field `5`: `string kind` (`interactive-agent` or `claude-remote-control` in v1)
   - field `6`: `string spawn_mode`
   - field `7`: `uint32 capacity`
   - field `8`: `string permission_mode`
   - field `9`: `string display_name`
   Account identity stays in the existing `AgentLaunchRoute`; it must match `provider`.
4. `SessionInfo` additions:
   - field `8`: `ManagedSessionInfo managed`
   - field `9`: `MemoryMeasurement process_memory`
5. `SessionList` additions:
   - field `3`: `MemoryMeasurement host_available_memory`
   - field `4`: `uint64 daemon_min_available_bytes`
   - field `5`: `uint64 collected_at_epoch_millis`
6. `ManagedSessionInfo` contains schema version, provider, opaque account id, project root, launch kind, managed launch id, and PTY generation. It must not contain config paths or credentials.
7. `MemoryMeasurement` uses explicit presence rather than sentinel zero:
   - `enum status` = `MEASURED | UNAVAILABLE | UNSUPPORTED`
   - `optional uint64 bytes`
   - `string provenance` = `linux-proc-memavailable` or `linux-proc-smaps-rollup`
   - `string diagnostic_code` from a fixed non-secret vocabulary.
8. New correlated `ManagedSessionLifecycleRequest` and `ManagedSessionLifecycleResponse` messages (next free top-level oneof field numbers): request includes action (`STOP`, `RESTART`), exact session id, expected generation, launch id, provider, account id, and project root. Restart returns the replacement session id/generation and explicit stop/start phases. Attach continues using `AttachSession.expected_generation` and the existing expected agent binding.
9. List/open/lifecycle handlers require negotiated `managed-agent-fleet-v1`; old peers fail closed. A lifecycle request never falls back to `HostExec` or agent process signal.

Server handler sequence for a managed open:

1. Validate capability, schema, provider/account equality, project root, launch kind, and bounded options.
2. Resolve the opaque account on the daemon and canonicalize the project on that host.
3. Deduplicate by the launch key; return the live existing managed session when the stable launch id is a retry.
4. Collect a fresh host-memory snapshot and evaluate the daemon/client effective headroom floor.
5. Create/register the managed PTY only after the gate allows it.
6. Deliver the generated provider entrypoint or official Claude Remote Control command through retry-safe startup input and persist only non-secret launch metadata.
7. Return the exact session id/generation.

### 5. Client API and call sites

The central integration adds:

- `Client::open_managed_agent_session(...) -> SessionOpened`
- `Client::managed_session_lifecycle(...) -> ManagedSessionLifecycleResponse`
- decoding helpers for explicit memory statuses
- capability checks in spawn, fleet refresh, and action routing

Call sites:

- Spawn card: a dedicated managed/Claude Remote Control choice produces one stable launch intent and uses opaque account routing. A normal remote agent launch remains unchanged.
- Cockpit model refresh: joins `SessionInfo.managed` to agent inventory by exact PTY id/generation and daemon host id.
- Cockpit pane: renders the compact detail projection and dispatches start/stop/restart/attach. Sidebar receives at most a managed-state icon.
- Workspace attach: reuses generation-checked native session attach. The official Claude mobile flow is reached by attaching to the managed PTY; no pairing data is copied into Zaplex state.

### 6. Security boundary

- No new listener or bind address is introduced. All management requests use the authenticated remote-server channel.
- The daemon resolves account routes locally; config directories and process environments never cross the wire.
- `/proc/<pid>` is read only after daemon ownership/generation validation. Arbitrary client PIDs are not accepted.
- Diagnostics use fixed codes. Raw I/O errors, `/proc` content, argv, environment values, sharing URLs, and transcript data are not protocol/UI/log fields.
- Stop/restart use exact daemon session ownership and generation, not a PID supplied by the client.

## Testing and validation

- `fleet_memory_tests.rs` covers valid `MemAvailable`, valid PSS, units, overflow, malformed/missing/permission-like reads, process disappearance, deduplication, partial-tree failure, and non-Linux unsupported projection (Behavior 8-15).
- `managed_fleet_tests.rs` covers identity validation, launch-key equality, provider capability, bounded official Claude argv, shell-sensitive names remaining argv data, effective policy floors, threshold equality, below-floor denial, stale/unavailable/unsupported fail-closed behavior, and diagnostic redaction (Behavior 1-2, 8-19, 23, 25).
- Existing `server_model_tests.rs` gains managed GC exemption for both age and ring-cap phases; explicit close and reader EOF still reap managed sessions; daemon grace does not arm with a managed PTY (Behavior 3, 5, 24).
- Protocol tests encode new fields and decode legacy messages; client tests verify capability gating and exact lifecycle envelopes (Behavior 2, 4-7, 13-15, 23).
- Workspace/Cockpit tests exercise exact Host × Account × Project × Provider routing, idempotent launch retry, generation mismatch, disconnect/reconnect attach, restart replacement identity, process-end display, and compact unknown-memory rendering (Behavior 1-7, 20-24).
- Static security assertions verify no managed-fleet response field or diagnostic contains `config_dir`, environment data, command lines, tokens, or pairing URLs; listener configuration is unchanged (Behavior 16-19, 25).
- No local Cargo invocation is used for this project. CI runs the focused remote-server, Cockpit, workspace, and protocol suites plus the required workspace `cargo check`.

## Risks and mitigations

- **PSS permission/race failures:** fail closed for new managed starts and render unavailable; never undercount a partial process tree.
- **Duplicate launches after reconnect:** stable launch id plus daemon launch-key registry makes open retry idempotent.
- **Stale lifecycle actions:** exact session id/generation and route identity are required server-side.
- **Managed sessions consuming unbounded memory:** the configured `MemAvailable` floor blocks new starts. Zaplex does not silently kill healthy sessions; explicit lifecycle controls remain available.
- **Claude CLI flag changes:** construct only the documented v1 flags and surface launch errors from the PTY. Zaplex does not couple to Claude's private Remote Control protocol.
