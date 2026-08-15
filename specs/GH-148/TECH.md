# In-App Subscription Agent — Technical Design

## Scope and source of truth

This design implements `PRODUCT.md` for GitHub issues #147 and #149–#153. The issue acceptance criteria are the approved product requirements for this implementation.

## Existing system

- `app/src/workspace/view/spawn_card.rs` hardcodes Claude and Codex model names and stores the selected string in spawn-card state.
- `app/src/terminal/cli_agent.rs` detects CLI accounts and already builds account-scoped launch environments that remove provider API-key variables.
- `app/src/ai/agent_sdk/driver/harness/claude_code.rs` starts Claude with an unstructured terminal harness and `--dangerously-skip-permissions`.
- `app/src/ai/blocklist/controller/response_stream.rs` dispatches in-app turns exclusively through `ai::agent_providers`.
- `app/src/ai/agent_providers`, `ai/byop_compaction`, `ai/byop_readiness`, `settings::ai::AgentProvider`, and the agent-provider settings widget implement the retired BYOP path.
- The existing blocklist conversation model already persists messages and renders text, reasoning, tool cards, usage, completion, and errors from `warp_multi_agent_api::ResponseEvent` values.

## Protocol boundaries

### Claude Code

Zaplex starts the selected official `claude` executable with piped stdin/stdout using the repository `command` crate. The process uses:

- `--input-format stream-json`
- `--output-format stream-json`
- `--verbose`
- `--permission-mode default`, with each structured permission request answered by the in-app approval UI
- the selected model and optional effort only when reported as supported
- `--session-id` for a new session or `--resume` for an existing session

`CLAUDE_CONFIG_DIR` is pinned to the selected account directory. `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` are removed. Startup sends the structured `initialize` control request. The response supplies the account summary, exact model identities, effort capabilities, and native session metadata. Incoming `can_use_tool` control requests become approval events; decisions are returned as `control_response` frames. Interrupt and model changes use control requests supported by the initialized protocol.

### Codex

Zaplex starts `codex app-server --listen stdio://` with piped stdin/stdout through the `command` crate. `CODEX_HOME` is pinned to the selected account directory and `OPENAI_API_KEY` is removed.

The JSONL client performs `initialize`, sends `initialized`, then calls `account/read` and `model/list`. Conversations call `thread/start` or `thread/resume`, `turn/start`, and `turn/interrupt`. Server notifications and requests are normalized into text, reasoning, item/tool/diff, approval, usage, completion, and error events. Missing required methods or incompatible initialization produces an upgrade error and never a shell fallback.

The client uses tolerant typed envelopes and explicitly modeled fields required by the installed protocol. Unknown notifications are logged and ignored; unknown requests receive a method-not-supported response so the server cannot hang.

## Proposed modules

Create `app/src/ai/subscription_agent/`:

- `types.rs`: `SubscriptionAgent`, account/host/installation identities, exact `ModelCapability`, target, session identity, normalized events, approvals, and the ten-state lifecycle enum.
- `process.rs`: local and remote structured-process launcher abstraction. Production uses `command::r#async::Command`; tests use an in-memory fake. Remote launch reuses the existing SSH command construction and preserves stdio JSONL framing.
- `claude.rs`: Claude initialization, capability discovery, session I/O, event parsing, approvals, interrupt, and exit handling.
- `codex.rs`: Codex app-server RPC IDs, initialization, capability discovery, thread/turn lifecycle, event parsing, approvals, interrupt, and exit handling.
- `catalog.rs`: asynchronous capability cache keyed by agent, account, host, executable identity, and CLI version. A refresh atomically replaces the model set and invalidates selections not present by exact ID.
- `router.rs`: deterministic target resolution for zero, one, or multiple agents/accounts using explicit stored preferences only.
- `response_adapter.rs`: maps normalized events to the existing persisted `ResponseEvent`/blocklist contract while exposing lifecycle state and pending approval handles.

## State and data flow

```text
host/account detection
        |
        v
CLI capability catalog -----> spawn card / model picker
        |                              |
        +---------- target ------------+
                       |
                       v
                    router
                       |
                       v
          Claude transport | Codex transport
                       |
              normalized events
                       |
                       v
       response adapter + lifecycle model
                       |
                       v
       persisted blocklist conversation UI
```

The selected model is a structured identity, not a display-name string. Context-window, effort, modality, and availability metadata remain attached to that identity. Catalog refresh compares exact IDs and clears a stale target before another turn can be sent.

## Routing rules

1. Filter to installations whose discovery result is ready and signed in.
2. Apply an explicitly stored agent preference when it resolves to a reachable installation.
3. If exactly one agent remains, select it; if multiple remain with no valid preference, return `NeedsAgentChoice`.
4. Within an agent, apply an explicitly stored account identity. If exactly one account remains, select it; otherwise return `NeedsAccountChoice`.
5. Apply a valid exact model preference. If absent, use only a model explicitly marked default by that CLI; otherwise return `NeedsModelChoice`.
6. Never rank accounts by inferred cost, plan, quota, or model-name heuristics.

## UI integration

- Replace `SpawnCard::models_for` with the catalog for its selected account and host. Loading, signed-out, incompatible, empty, and error results render as explicit non-selectable states.
- Add a target header to the blocklist agent view with agent, account, host, working directory, session, and exact model. It collapses labels but not required actions in narrow layouts.
- Derive composer enablement and primary actions from the lifecycle enum. Approval and recoverable-error panels occupy conversation content space rather than floating over the header or composer.
- Remove provider and `/model` BYOP surfaces. Model selection routes to the dynamic target selector.

## BYOP retirement and migration

Remove the BYOP dispatch and provider implementation after both subscription transports feed the response adapter. Then remove:

- agent-provider settings types, models, preferences, handlers, and widget;
- `AgentProviderSecrets` and its secure-storage key;
- BYOP readiness and compaction modules and controls;
- BYOP-only LLM identifiers, placeholders, test fixtures, translations, and documentation;
- the `genai` dependency when the final reference audit confirms no non-BYOP consumer.

Settings deserialization retains ignored compatibility fields for one release or uses serde defaults/aliases so old files load. Serialization omits retired fields. Startup performs idempotent best-effort deletion of the old secure-storage value and records no replacement secret.

## Persistence and sessions

The existing conversation/message persistence remains authoritative for UI history. Add optional structured target and native-session metadata to the conversation record using the established migration path. Native Claude session IDs and Codex thread IDs are stored only after the CLI reports or accepts them. Resume always uses the stored agent-specific identifier and verifies that the installation/account identity still matches.

## Security

- Never open `auth.json`, Claude credential files, or equivalent token stores.
- Never log process environments, protocol fields that are marked sensitive, or raw authentication errors containing secrets.
- Remove provider API-key variables in addition to pinning the account home.
- Use the `command` crate for every subprocess.
- Use argument vectors and the existing SSH quoting helpers for remote execution.
- Keep approval mode manual; do not add permission bypass flags.

## Failure and cancellation semantics

- Process startup, initialization timeout, malformed JSON, EOF, non-zero exit, and host disconnect each produce a typed recoverable error.
- Cancellation sends the native interrupt operation first and terminates the child only after a bounded grace period.
- Dropping an active transport closes stdin and kills the owned child to avoid orphan processes.
- Pending approval requests are rejected on cancellation or disconnect.
- A known native session identifier remains resumable after recoverable transport failure.

## Implementation sequence

1. Add shared types, catalog, router, process abstraction, and unit tests.
2. Add Claude discovery/session transport and protocol fixture tests.
3. Add Codex app-server discovery/session transport and protocol fixture tests.
4. Adapt both transports to the existing response/history pipeline and lifecycle state.
5. Replace spawn-card model selection and add the target/session UI.
6. Remove BYOP runtime/settings/secrets/dependencies and add compatibility cleanup.
7. Run the static policy checks and CI validation authorized for this repository.

## Verification matrix

| Product invariant | Verification |
| --- | --- |
| 1–2 | Catalog tests cover exact IDs, metadata, cache keys, refresh, and stale-selection invalidation for both protocol fixtures. |
| 3–5 | Launch-descriptor tests assert account homes, scrubbed variables, structured flags, and absence of token-file/API paths or permission bypasses. |
| 6–9 | Router and session tests cover target completeness, ambiguity, real identifiers, resume, restart, end, and cancel. |
| 10–12 | Lifecycle transition tests plus normal/narrow UI render tests cover all ten states and action reachability. |
| 13–14 | Repository reference audit, settings compatibility tests, secure-secret cleanup test, and dependency audit verify BYOP retirement. |
| 15 | Protocol fixture tests cover missing CLI, signed-out, incompatible version, disconnect, malformed event, process exit, retry, and resume. |

Repository policy forbids local Cargo builds. Local verification therefore uses formatting/static checks such as `git diff --check`, targeted source audits, and the repository's Phase A readiness script; compile and test execution belongs to GitHub Actions after explicit approval.

## Risks

- Both CLI protocols evolve. The version-keyed catalog and tolerant envelopes isolate changes, while required-method checks fail closed with an upgrade message.
- Remote stdio can be disrupted by shell startup output. The remote launcher must use the existing non-interactive SSH path and treat pre-protocol stdout as a typed initialization error.
- BYOP removal touches broad settings and test surfaces. It lands only after subscription dispatch works and is verified with a final `rg` reference/dependency audit.
