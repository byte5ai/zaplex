# GitHub issue and pull-request flows

## Summary

Zaplex exposes its GitHub issue and pull-request workflows as real, repository-scoped actions. A user can start an issue analysis, a pull-request analysis, or a pull-request review from the Cockpit surfaces and complete the flow with the subscription account they choose, while every GitHub mutation remains an explicit human decision.

Figma: none provided. The existing Cockpit, Spawn Card, agent terminal, and native confirmation surfaces define the visual language.

## Behavior

1. The available workflows have stable identities independent of translated labels: quick issue, issue triage, and pull-request review.

2. Starting a workflow freezes a repository identity consisting of its GitHub slug and exact working-tree directory. Every later list, analysis, and mutation in that flow uses that same identity; changing the active tab or directory cannot retarget an in-progress flow.

3. A workflow is available only when its directory resolves to a Git worktree with a GitHub remote. A missing directory, non-Git directory, non-GitHub remote, or ambiguous remote produces a visible error and performs no action.

4. Quick issue starts an agent task scoped to the frozen repository and asks for a structured issue draft. The agent must inspect the actual worktree and must not create the issue itself before the user reviews the draft.

5. Issue triage first retrieves the open-issue list for the frozen repository. The user sees a loading state while this happens, a distinct empty state when there are no open issues, and a visible actionable error when `gh`, authentication, networking, or GitHub fails.

6. Pull-request analysis and review first retrieve the open-PR list under the same loading, empty, and error-state contract as issue triage.

7. The selected issue or PR is represented by repository slug plus GitHub number, not by title or list position. Refreshes, duplicate titles, sorting changes, and active-worktree changes cannot change the selected target.

8. Analysis runs on an explicitly selected Claude or Codex account when the user selects one, or on Zaplex's freest suitable account when the user leaves account routing automatic. Unavailable, unreadable, or overcommitted accounts are never silently treated as free.

9. Analysis output has a structured verdict. PR review includes a summary, approve/comment/request-changes decision, and zero or more file-and-line findings. Issue triage includes type, priority, actionability, optional comment, and optional close recommendation. Malformed or empty output is a visible retryable error and never enables a mutation.

10. Approve, comment, request changes, merge, issue creation, issue comment, and issue close each require a confirmation that names the repository, target, action, and exact body/title where applicable.

11. Cancelling or dismissing a confirmation performs no GitHub mutation. A confirmation token applies to one frozen operation only and cannot be reused for another target or changed content.

12. Mutations run as argument vectors through the GitHub CLI rather than by concatenating an untrusted shell command. Titles, bodies, labels, repository slugs, and comments remain literal arguments even when they contain quotes, newlines, substitutions, or shell metacharacters.

13. A multi-step close with a comment is fail-closed: if posting the comment fails, Zaplex does not close the issue. Other mutations are single GitHub operations and never retry automatically after an indeterminate failure.

14. Authentication, executable-not-found, network, rate-limit, permission, and GitHub validation failures remain visible with their original useful detail. Zaplex never reports success or advances the flow after a failed command.

15. Successful mutations show the resulting URL or a concise success result and leave the analysis visible for reference.

16. The workflows are reachable from the Command Palette and from any Cockpit/Spawn Card entry that supplies a repository scope. Keyboard selection and Enter execute the same stable flow target as pointer activation.

17. On platforms where local GitHub CLI execution or subscription-agent routing is unavailable, the workflows are capability-gated and do not appear as working actions.
