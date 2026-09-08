# Agent conversation lifecycle and layout — technical design

GitHub: https://github.com/byte5ai/zaplex/issues/153

## Current state

`AgentLifecycle` already models the ten required states in `app/src/ai/subscription_agent/types.rs`. Runtime routing stores lifecycle, target, session, approval, and unresolved agent/model choices in `SubscriptionSessionRegistry`.

The missing boundary is presentation and enforcement:

- `AgentLifecycle::accepts_prompt()` is currently exercised only by unit tests.
- `Input::render_agent_input` always renders the editable composer.
- `Input::submit_ai_query` does not recheck subscription lifecycle before dispatch.
- The footer formats all target identity fields as one unbounded string and reduces a recoverable error to `Retry available`, discarding the diagnostic.
- Ended and unrecoverable sessions have no explicit route to a new conversation or shell.
- `/model` still opens the legacy global/BYOP selector inside a subscription-agent conversation.

## Design

### One presentation policy

Add `app/src/ai/subscription_agent/presentation.rs` as a pure mapping from `AgentLifecycle` to `ConversationPresentation`. It owns:

- status label and safe detail text;
- whether the composer accepts a prompt;
- the reason shown when it does not;
- the valid lifecycle actions;
- stable identity fields for agent, account, host, working directory, model, session, and status.

The mapping is exhaustive. UI code does not reproduce lifecycle matches for composer policy or action availability.

An active Agent View with no subscription registry entry remains the initial routing state and is not blocked: the first prompt is routed only to the subscription-agent resolver and never to the shell. Once a lifecycle exists, the presentation policy is authoritative. This preserves the current discovery protocol while preventing stale or ended sessions from accepting prompts.

### Composer enforcement

Add a small `Input` helper that obtains the active Agent View conversation and its subscription lifecycle. `render_agent_input` uses the presentation policy to replace the editor with an in-flow disabled-state notice when prompts are not accepted. The editor model is not cleared or mutated, preserving the draft.

`submit_ai_query` calls the same helper before aborting attachments, altering input state, or dispatching. A blocked state returns immediately. This is the correctness boundary; rendering is explanatory rather than the only guard.

### Footer and responsive identity

Replace the single concatenated target string with a wrapping sequence of identity fields. Each field has a stable label and value, so normal width forms a compact row and narrow width wraps at field boundaries. Lifecycle detail and actions share the same wrapping footer flow and do not use absolute positioning.

Recoverable error text is normalized to one bounded line before display. Approval actions remain tied to the stored request ID. Completed/recoverable sessions retain their existing exact-session resume path. Ended and non-resumable states expose explicit `New conversation` and `Back to shell` actions through existing terminal actions.

### Model command boundary

When the active Agent View conversation has subscription-agent state, `/model` must not open the legacy global selector. Unresolved subscription model choices continue to use the models supplied by `SubscriptionSessionRegistry`. The legacy selector remains unchanged outside subscription conversations.

## Files

- `app/src/ai/subscription_agent/presentation.rs`: pure lifecycle, action, diagnostic, and identity presentation policy.
- `app/src/ai/subscription_agent/presentation_tests.rs`: exhaustive state and layout data tests.
- `app/src/ai/subscription_agent/mod.rs`: module exports.
- `app/src/terminal/input/agent.rs`: in-flow unavailable-composer rendering.
- `app/src/terminal/input.rs`: last-moment submission guard and shared active-conversation lookup.
- `app/src/terminal/input/slash_commands/mod.rs`: subscription-aware `/model` boundary.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`: responsive identity/status blocks and lifecycle actions.
- `docs/ui/agent-conversation-lifecycle.html`: deterministic normal, narrow, and long-content UI contract.

## State invariants

1. `Ready` and `TurnCompleted` are the only lifecycle states that accept a prompt.
2. A blocked submit changes neither the draft nor pending attachments and emits no AI query.
3. Resume acts on the session identity stored for the same conversation ID.
4. Approval acts on the request ID stored for the same conversation ID.
5. Ended sessions have no resume or approval action.
6. Recoverable errors with a session may resume/restart/end; errors without a session may start over or return to shell.
7. Lifecycle and target data are always looked up by the currently active conversation ID.
8. Subscription `/model` never opens the legacy selector.

## Verification

- One unit test per lifecycle variant validates label, composer policy, detail, and actions.
- Unit tests validate identity field order and normal/narrow grouping with long values.
- Registry/input helper tests validate conversation isolation and draft-preserving blocked submission policy.
- Existing router tests continue to prove a prompt has one destination; add a focused assertion for active subscription routing if the existing coverage does not express it directly.
- `rustfmt --check` and `git diff --check` run locally; build and Rust tests run only in GitHub Actions per project policy.
- Pull-request CI must pass `cargo check`, focused test gates, and repository policy checks.

## Risks and mitigations

- Registry state uses interior mutability and does not independently emit UI events. Runtime lifecycle transitions already coincide with transcript/controller updates that invalidate Agent View; the submit guard remains correct even if a frame is stale.
- Session and path values can be long. Field-level wrapping and bounded diagnostics prevent one value from displacing actions.
- The initial prompt still initiates discovery. It remains an agent-only route; subsequent prompts are governed by explicit lifecycle state.
