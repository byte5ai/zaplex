# In-App Subscription Agent

## Summary

Zaplex lets a signed-in user run the Claude Code or Codex subscription already available on the selected local or remote host. The in-app agent is a transparent router and conversation surface around the installed official CLI; it is not an independent model provider.

This specification covers GitHub issues #147 and #149–#153. GitHub issue #148 is the parent epic.

Figma: none provided.

## Problem

The current in-app agent mixes static model assumptions with a bring-your-own-provider (BYOP) path. Static model IDs become stale, provider keys create a second authentication and billing path, and the UI cannot accurately explain which agent, account, host, working directory, model, or session will handle a turn.

Users need one honest path that uses their installed Claude Code or Codex subscription, exposes the capabilities reported by that installation, and preserves the native agent session and approval semantics.

## Goals

- Discover accounts, models, reasoning controls, and protocol support from each installed CLI.
- Run Claude Code and Codex through their official structured protocols.
- Route a prompt to an explicit reachable agent/account/host/working-directory target.
- Preserve real sessions, tool activity, approvals, usage, cancellation, and recoverable failures.
- Remove API-key-based agent providers and their settings, secrets, preferences, and runtime paths.
- Keep the normal and narrow layouts stable throughout all lifecycle states.

## Non-goals

- Reading Claude or Codex credential files or tokens.
- Sending model requests directly to Anthropic, OpenAI, or another model API.
- Reimplementing an agent loop or silently falling back to a shell command.
- Installing, upgrading, or signing in to a CLI without an explicit user action.
- Combining Claude Code and Codex in one turn.

## User-facing invariants

1. The model picker shows only models returned by the selected installed CLI for the selected account and host. Each choice uses the exact reported model identity and capability metadata.
2. A previously selected model that is no longer reported is invalidated and requires a valid replacement; Zaplex never invents a fallback model ID.
3. Claude Code runs with the selected account's `CLAUDE_CONFIG_DIR`, with Anthropic API-key environment variables removed, and communicates through structured stream JSON with manual approval handling.
4. Codex runs with the selected account's `CODEX_HOME`, with `OPENAI_API_KEY` removed, and communicates through `codex app-server` after a successful protocol capability check.
5. Zaplex never reads or displays a Claude or Codex token and never sends the turn directly to a model-provider HTTP API.
6. A target is identified by agent, account, host, working directory, and model. The composer is enabled only while that target is reachable and able to accept input.
7. With one reachable agent, the router selects it. With both agents reachable, the router uses the stored default or asks for a choice. With multiple accounts, it uses the stored account or asks for a choice; no selection is based on an assumed pricing tier.
8. The conversation header continuously shows the selected agent, account, host, working directory, session, and model without covering navigation or composer controls.
9. New and resumed conversations use the CLI's real session/thread identifiers. Ending, resuming, restarting, and cancelling a session are explicit actions.
10. The UI represents these distinct states: no agent installed, not signed in, ready, starting, responding, running a tool, waiting for approval or input, turn completed and resumable, session ended, and recoverable error.
11. Tool calls, reasoning, text, diffs, usage, errors, and process exit are rendered from structured protocol events. Approval decisions are sent back through the same protocol.
12. Normal and narrow layouts remain usable in every state: controls do not jump on hover, overlays do not obscure required controls, and the primary session actions stay reachable.
13. BYOP provider settings, API-key fields, provider/model preferences, provider-specific slash commands, and BYOP-only readiness or compaction controls are absent.
14. Existing settings containing retired BYOP fields are tolerated during loading but are not written back. Stored BYOP secrets are deleted once and are not recreated.
15. If a CLI is missing, signed out, incompatible, disconnected, or exits unexpectedly, the UI names the affected target, preserves the resumable session when possible, and offers a relevant setup, retry, resume, or new-session action.

## Primary flows

### Start a conversation

1. Zaplex inspects the selected host for supported Claude Code and Codex installations.
2. It queries each reachable CLI for version, account, and model capabilities.
3. The router resolves a unique target or asks the user to choose the unresolved agent/account/model dimension.
4. The header displays the complete target and the composer becomes active.
5. Sending a prompt starts a native session/thread and streams structured events into the conversation.

### Approve a tool

1. The CLI requests permission through its structured protocol.
2. Zaplex enters the waiting-for-approval state and shows the tool, input, and available decisions.
3. The user's decision is sent to the pending CLI request.
4. The conversation returns to tool or response streaming, or records the denial.

### Resume or restart

1. A completed or interrupted conversation retains its native session/thread identifier.
2. Resume reconnects through the agent's official resume operation.
3. New session discards the active target's session association only after explicit confirmation when work may be lost.

## Error behavior

- Discovery failures do not expose stale models as valid choices.
- An unsupported Codex app-server or Claude structured-stream protocol reports an upgrade action instead of using an unstructured fallback.
- Authentication failures identify the selected account directory and provide a sign-in action without exposing credentials.
- Remote disconnection keeps the conversation readable and marks the session resumable when its identifier is known.
- Process exit and malformed protocol messages become recoverable errors with diagnostic context safe for display.

## Success criteria

- Every invariant above has unit or integration coverage, or a documented static verification where process/UI integration cannot be exercised locally.
- No product path can dispatch an in-app agent turn through a BYOP HTTP provider.
- No in-app Claude path uses `--dangerously-skip-permissions`.
- Model/account selections are derived from and keyed by the selected CLI installation identity and version.
