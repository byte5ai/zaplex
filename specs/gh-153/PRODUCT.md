# Agent conversation lifecycle and layout

GitHub: https://github.com/byte5ai/zaplex/issues/153

## Summary

The in-app Agent conversation must explain where a prompt will go, what the selected agent is doing, and which actions remain valid at every point in the conversation. The composer, status, transcript, and follow-up actions form one stable layout instead of competing overlays.

Figma: none provided. The implementation contract is documented in `docs/ui/agent-conversation-lifecycle.html`.

## Goals

- Represent all ten conversation lifecycle states with distinct, understandable presentation.
- Prevent prompt submission whenever no reachable destination can accept it.
- Keep agent, account, host, working directory, session, model, and lifecycle identity visible without breaking narrow layouts.
- Make approval, recovery, resume, new-conversation, and return-to-shell actions explicit.
- Preserve the user's draft when a lifecycle transition temporarily disables the composer.
- Remove the old BYOP model-selector path from subscription-agent conversations.

## Non-goals

- Redesigning agent discovery or authentication owned by the agent-selection work.
- Replacing the subscription-agent protocol or transcript storage.
- Removing every legacy BYOP code path across the application; this issue only prevents it from surfacing in an active subscription-agent conversation.
- Claiming real-agent or signed-macOS runtime evidence that has not been executed.

## Product behavior

1. An active conversation has exactly one visible destination: the selected agent, never an ambiguous agent-and-shell combination.
2. The UI distinguishes these ten lifecycle states: no supported agent installed, installed but signed out, ready, starting, responding, tool running, waiting for approval or user input, turn complete and resumable, session ended, and recoverable error.
3. The composer accepts prompts only while the destination is ready or a completed turn can resume. Other states replace the editable composer with an in-flow explanation and valid next actions; existing draft text is retained.
4. Submission rechecks lifecycle state before clearing the draft, so stale UI or keyboard input cannot bypass the disabled state.
5. Streaming output, tool activity, and approval requests use stable transcript blocks. Status and actions take layout space and never cover the composer or each other.
6. A completed turn resumes the exact recorded agent session. It does not silently start a different agent, account, host, directory, or model.
7. An ended session offers `New conversation` and `Back to shell`. It does not show an active composer or imply that the ended session is still waiting.
8. Missing-agent and signed-out states provide a concrete next step without asking for a BYOP provider or API key.
9. Recoverable errors display a safe diagnostic message and the actions that can recover without discarding the transcript or draft.
10. The conversation identity includes agent, account, host, working directory, session, model, and lifecycle. Missing optional fields are omitted instead of rendered as empty placeholders.
11. At normal width, identity is a compact toolbar. At narrow width, it wraps into stable rows with bounded text and accessible full labels.
12. User messages, agent messages, tool activity, approvals, status notices, and errors share one visual block system with distinct semantics.
13. Menus and notices reserve layout space. Narrow and long-content states retain reachable primary actions and do not introduce dead click targets.
14. Enter submits only when the composer is enabled. Escape closes transient conversation UI before leaving the conversation. Approval, cancellation, and resume actions operate on the active conversation only.
15. Switching conversations recomputes presentation from the selected conversation; lifecycle, errors, approvals, and target identity never leak between sessions.
16. `/model` in a subscription-agent conversation must not open the legacy BYOP selector. Any subscription model choice is limited to models reported for that agent and account.
17. The HTML contract covers normal, narrow, and long-content variants and remains suitable for deterministic screenshot validation without private account data.

## Acceptance criteria

- Pure presentation tests cover all ten lifecycle states and their composer/action policy.
- Tests cover normal, narrow, and long identity layouts plus conversation switching.
- Input tests prove that blocked states retain the draft and dispatch no prompt.
- Action tests cover Enter, Escape, approval, cancellation, resume, new conversation, and back to shell.
- A routing test proves that one submission cannot be delivered to both shell and agent.
- The HTML contract documents the same states and responsive behavior implemented in Rust.
- Repository CI, including `cargo check`, is green for the pull request.
