# GitHub issue and pull-request flows

## Context

The product behavior is defined in [PRODUCT.md](./PRODUCT.md). `app/src/cockpit/github_flows.rs` currently owns typed verdict parsing, stable flow keys, prompts, and shell-string builders, but production code does not call it. The Cockpit launches contextual agent tasks through `WorkspaceView::open_spawn_card` in `app/src/workspace/view.rs`, while local processes must use the workspace `command` crate. The dynamic entry point added for GH-167 lives under `app/src/search/command_palette/cockpit/`.

The two refreshed references were inspected at claudeplex `8c2041ff68d97463aed7aeb01da0f16b708b8e22` and claudeplex-desktop `8c0aad0a944a8f5b6a26636d0827db57ca22d0f3`. Their strongest reusable properties are stable repository/number targeting, typed error envelopes, a freest-account analysis route, structured verdicts, and explicit mutation confirmation.

## Proposed changes

1. Extend `app/src/cockpit/github_flows.rs` into the authoritative headless flow contract:
   - `RepositoryContext` resolves and freezes an absolute worktree plus GitHub slug from Git metadata without trusting display labels.
   - `GitHubTarget` freezes repo plus issue/PR number.
   - typed issue/PR list rows parse bounded JSON returned by `gh`;
   - `GitHubOperation` represents every mutation as typed data;
   - `ConfirmedGitHubOperation` is created only by confirming the exact operation fingerprint;
   - argument-vector builders replace shell strings in production paths while legacy string helpers remain test-compatible until their callers migrate;
   - prompt builders include the frozen repo/target and demand the existing structured verdict schemas.

2. Add a small native executor in the same module. It calls `command::async::Command` with program plus arguments, a frozen `current_dir`, bounded output, and no shell. It maps spawn failure, non-zero status, malformed JSON, and empty output into typed visible errors. WASM exposes no executable capability.

3. Add `app/src/cockpit/github_flow_dialog.rs` as the productive native view. It owns the complete state machine: ready, loading target list, empty/error, issue/PR selection, read-only analysis, structured result, exact confirmation, mutation in progress, and success/error result. A monotonic generation rejects completions from a dialog instance that was closed and reopened for a different target.

4. The dialog routes analysis through the existing native subscription-agent protocol with one explicit installed Claude/Codex account or the automatic freest-account policy. It reads provider events directly, bounds collected output, denies every command/file approval request, and parses the final text into the existing typed schemas. Quick-Issue does not start until the user has had a chance to change the account route.

5. Analysis inputs are loaded by Zaplex before the model call. Issue/PR selection is validated against the current list state, detail/diff commands keep the selected GitHub number frozen, and prompts delimit GitHub bodies/diffs as untrusted data. The model never receives authority to mutate GitHub.

6. `GitHubOperation` is the only productive mutation path. The dialog renders its complete `confirmation_text`, and `ConfirmedGitHubOperation` is issued only when that exact text and operation still match. Triage comment and close are separate confirmations. `execute_confirmed` revalidates the repository immediately before each command and never substitutes the active repository.

7. Capability gating requires native process support, a valid GitHub worktree, an installed `gh`, and at least one healthy installed Claude/Codex CLI account. Palette and dialog revalidate capabilities independently so a stale row cannot silently no-op or retarget.

## Testing and validation

- `app/src/cockpit/github_flows_tests.rs` covers Behavior 1-3 and 7 with stable keys, GitHub-remote parsing, worktree identity, duplicate labels, and repo revalidation.
- Parser tests cover Behavior 5-9 with loading-result inputs, empty lists, PR filtering from the issues endpoint, bounded malformed JSON, fenced verdicts, and required fields.
- Operation/executor contract tests cover Behavior 10-14: each mutation type requires a matching fingerprint; cancellation produces no confirmed operation; hostile content remains one argv entry; and comment-before-close is fail-closed.
- `app/src/cockpit/github_flow_dialog_tests.rs` covers frozen Quick-Issue repositories, separate triage operations, exact review bodies, target mismatch rejection, and exhaustive operation-to-action mapping.
- Palette integration tests cover Behavior 16-17: flow targets include frozen repo identity, disappear outside GitHub repos, and keyboard acceptance dispatches the stable target.
- CI runs `cargo check` and the focused Cockpit/search suites. A manual pass on the signed build validates the routed-agent result presentation and confirmations with a disposable test repository.

## Risks and mitigations

- A repository can change while a flow is open. Stable slug plus absolute worktree revalidation prevents cross-repo mutations.
- GitHub command output is untrusted and potentially large. List commands request selected JSON fields and parsers enforce row/output bounds.
- Agent text can be malformed. Mutations remain disabled unless parsing yields a typed verdict and the user confirms an exact typed operation.
