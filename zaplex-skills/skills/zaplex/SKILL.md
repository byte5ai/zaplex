---
name: zaplex
description: Control the current Zaplex terminal from an agent or script. Use when a user asks to split a pane, open a Git worktree in a new pane, focus an existing local or fleet session, send text to another pane, or distribute work across Zaplex panes.
---

# Zaplex Control

Use Zaplex's authenticated local control surface. Never reproduce or expose the
control token or socket path.

## Check availability

Require `ZAPLEX_CONTROL_SOCKET`, `ZAPLEX_CONTROL_TOKEN`,
`ZAPLEX_SURFACE_ID`, and `ZAPLEX_TAB_ID` in the current environment. If any is
missing, stop and say that the request must run inside a Zaplex pane with the
control surface enabled.

## Control panes

- Split beside the current pane:
  `zaplex control split-pane --orientation right --dir /absolute/path`
- Create or attach a worktree and open an agent-ready pane:
  `zaplex control open-worktree-in-pane --repo /absolute/repo --branch feature/name`
- Focus a pane returned by another control command:
  `zaplex control focus-session --surface-id <surface-id>`
- Focus a fleet session:
  `zaplex control focus-session --host <host> --session-id <session-id>`
- Place text in a pane without submitting:
  `zaplex control send-text --surface-id <surface-id> --text '<text>'`
- Place and submit text:
  `zaplex control send-text --surface-id <surface-id> --text '<text>' --submit`

`split-pane` and `open-worktree-in-pane` print the new surface ID. Capture that
exact output before sending text or changing focus.

When `open-worktree-in-pane` creates a new branch, Zaplex uses the repository's
explicit default branch (`origin/HEAD`, then a known main branch) as its base.
It never branches from the caller's incidental current `HEAD`.

## Workflows

For one worktree per branch:

1. Resolve the repository to an absolute path.
2. Call `open-worktree-in-pane` once per distinct branch.
3. Record each returned surface ID.
4. Send the branch-specific task to that surface with `send-text --submit`.
5. Report the created branches and surfaces.

For a test watcher:

1. Split the current pane into the requested direction.
2. Capture the returned surface ID.
3. Send the exact watcher command only after the user has authorized that
   command.

## Safety

- Use absolute repository and directory paths.
- Never invent a surface, tab, host, session, or branch ID.
- Do not send destructive commands or submit text unless the user authorized
  that action.
- Keep each worktree on a distinct branch.
- Treat authentication or `unauthorized` errors as hard failures; do not retry
  with altered credentials.
