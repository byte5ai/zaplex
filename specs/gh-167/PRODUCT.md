# Dynamic Cockpit Command Palette source

## Summary

The Command Palette provides one keyboard-first index over Cockpit accounts, live and dormant agent sessions, connected hosts and projects, and repository-scoped GitHub workflows. Results follow the live Cockpit inventory and always execute against stable object identities.

Figma: none provided. Results reuse the existing Command Palette row, selection, focus, and accessibility design.

## Behavior

1. When Cockpit is enabled, an unfiltered Command Palette query searches Cockpit accounts, sessions, connected hosts, projects, and available GitHub workflows alongside existing sources.

2. Account results contain provider and account label in their searchable text. Accepting one opens or focuses the exact account detail identified by its stable account key, even when two accounts share a display label or email.

3. Session results include live sessions from the connected fleet and bounded dormant local sessions. Searchable text can contain provider, account label/email, host, project, session name/ID, branch/worktree, and model, but never secrets, config-directory paths, or transcript contents.

4. Accepting a live session focuses its known pane or attaches to that exact Host × Provider × Account × Session identity. Accepting a dormant local session resumes that exact conversation. A live session that cannot be safely attached remains unavailable rather than launching a duplicate.

5. Waiting sessions rank ahead of otherwise equivalent active, monitor, or idle sessions. Match quality still matters, so an unrelated waiting row does not outrank an exact match.

6. Host results exist only for the local root and currently connected remote roots. Accepting a host opens the Spawn Card with that stable host identity preselected.

7. Project results are scoped by stable host identity plus project root. Accepting one opens the Spawn Card with both host and project preselected. Duplicate host/project display names remain independently addressable.

8. GitHub workflow results exist only when the active project resolves to a GitHub-backed worktree. Accepting one starts the exact stable flow key scoped to the frozen repository identity.

9. Results refresh while the palette remains open. Accounts/sessions added by a newer Cockpit generation appear; removed accounts, disconnected remote hosts, closed sessions, and removed projects disappear without requiring the user to close and reopen the palette.

10. Refresh is generation-safe. A slow result built from an older inventory can never replace results produced from a newer inventory.

11. An action is offered only when all capabilities required to execute it are currently available. Cockpit disabled, disconnected host, unsupported remote inventory, missing resume identity, non-GitHub repository, and unavailable GitHub/agent support remove or disable the corresponding action instead of producing a silent no-op.

12. Search and deduplication use stable object keys. Duplicate display names, duplicate session IDs on different accounts/hosts, and identical project names never collapse distinct targets.

13. All result classes work with existing keyboard navigation: typing filters, Up/Down move the active row, Enter executes it, and Escape closes the palette. Pointer activation executes the same action.

14. Each result has an accessibility label that identifies its object class, primary label, and scope, plus an Enter help message.

15. Cockpit results do not expose full local paths in visible secondary text unless the path is the user-chosen project label; stable config directories remain internal routing data only.

## Goals

- Make every Cockpit routing object reachable without leaving the keyboard.
- Preserve identity and capability boundaries during live inventory churn.
- Reuse the existing Command Palette rather than introduce a second search surface.

## Non-goals

- Indexing transcript contents.
- Showing offline favorite hosts as Cockpit roots.
- Replacing the dedicated Cockpit pane or Spawn Card.
