# Cockpit, Connections, and account identity

GitHub: https://github.com/byte5ai/zaplex/issues/160

## Summary

Zaplex separates configured host connections from the live AI-session Cockpit while keeping both
surfaces on one stable host identity model. The Cockpit becomes a quiet live
`Host → Project → PTY session → Agent` tree, and the large Claude and Codex panes show provider,
account, usage, and attention without duplicated identity or redundant status text.

## Figma

Figma: none provided. The binding visual reference is
[`docs/ui/cockpit-sidebar-connections.html`](../../docs/ui/cockpit-sidebar-connections.html).

## Goals

- Keep connection configuration, favorites, and live session observation distinct and predictable.
- Make local and connected-remote AI work visible without mirroring offline registry hosts.
- Restore truthful Claude history and account discovery behavior.
- Minimize sidebar noise while preserving unambiguous provider, state, and accessibility semantics.

## Non-goals

- No new app-wide icon rail or navigation system.
- No provider level between host and project.
- No second connection registry or account-override store.
- No runtime dependency on either reference repository.
- No offline registry hosts in the live Cockpit tree.

## Behavior

1. **Connections is an independent sidebar element.** It lists every registered SSH host and owns
   adding, editing, deleting, connecting, disconnecting, and favoriting hosts. The Cockpit never
   duplicates those configuration controls.

2. **The Connections list is quiet by default.** Host labels use the same neutral treatment. A
   trailing connection control communicates state without prose: a connected host uses an intact
   green plug and a disconnected host uses a muted slashed plug. The list does not repeat labels
   such as “local,” “not connected,” or “2 open,” and it has no explanatory legend or live-root
   summary.

3. **Favorites project into the tab `+` menu.** The menu reads stable host references from the
   Connections registry and shows favorite hosts only. Its first level contains only the favorite
   mark, host label, and submenu chevron; launch actions appear in the submenu. Editing a favorite
   never creates a second connection record.

4. **The Cockpit tree hierarchy is `Host → Project → PTY session → Agent`.** A project represents
   the detected repository/worktree grouping. A PTY session is the Zaplex terminal-session
   container when that identity exists. Each agent leaf represents one Claude, Codex, or other
   supported agent conversation. Agent conversations without PTY metadata receive separate stable
   fallback session containers and are never merged by label alone.

5. **Local remains visible.** The local host root and its discovered live sessions are always part
   of the Cockpit. With no local agent sessions, the root remains and shows an honest empty state.

6. **Remote roots follow actual open Zaplex sessions.** Opening the first session to a remote host
   adds exactly one root keyed by stable host identity. Further sessions reuse it. Closing or
   disconnecting the final open session removes the root synchronously without deleting the host
   or changing its favorite state in Connections.

7. **Connection and inventory are different states.** A connected remote root remains visible
   while its AI inventory is honestly empty, temporarily unavailable, or unsupported by an older
   peer. None of those states is reported as “no agent installed” or as a disconnected host.
   Results from an older in-flight inventory generation cannot re-add a root after the final
   disconnect.

8. **Tree expansion is explicit and stable.** Hosts, projects, and PTY sessions start expanded and
   expose chevrons. Zaplex never auto-collapses calm hosts because the fleet grew. Hover may change
   color only; it cannot reveal content or reflow a row.

9. **The expanded tree uses one visual mechanism per fact.** Host and agent state use state glyphs;
   hierarchy uses chevrons and indentation. Project and PTY-session rows do not gain redundant
   state labels. Container counts are hidden while their row is expanded and appear only when
   collapsed. A collapsed count may turn amber when it hides a waiting descendant.

10. **Agent leaves are compact and provider-explicit.** Each leaf visibly names `Claude`, `Codex`,
    or the relevant provider, followed by the model when known. Context percentage, cost, account
    email, effort, activity age, and other metrics stay in the selected detail pane rather than a
    fixed sidebar metric column.

11. **Tree state is glyph-only.** Waiting, working, and idle agent leaves do not repeat visible
    words such as “waiting,” “active,” or “idle.” The glyph has an accessible state label or
    tooltip. Detail tables retain their explicit status word because they are the semantic detail
    surface.

12. **Waiting attention is visible but restrained.** Only the amber waiting glyph pulses. Its
    1.6-second cycle combines a modest core-brightness change with an expanding ring capped at
    approximately twice the glyph footprint. Working, idle, host-connection, and other glyphs are
    static. Reduced-motion mode replaces the animation with a static amber emphasis.

13. **Aggregate attention does not add prose.** The tree section header uses the amber waiting
    glyph plus a count when attention exists. It does not add “N waiting.”

14. **Selecting an agent is exact.** Clicking a leaf attaches to or resumes that exact agent using
    the existing stable local/remote route. Collapsing or expanding a row never changes routing,
    account association, waiting order, guardrails, or capability gates.

15. **Account identity has one hierarchy on every surface.** Sidebar account cards and large
    account panes use the provider (`Claude` or `Codex`) as the headline. The subordinate line
    contains the account label or email plus the plan when known. A label identical to its email is
    rendered once. Provider identity is never communicated only by color or icon, and a separate
    provider strip does not repeat the same heading above the card.

16. **Account cards retain both subscription windows.** Each sidebar account card shows the
    five-hour and weekly usage meters. The large pane may add reset times, tokens, and cost
    provenance without duplicating the identity heading.

17. **Sessions needing the user are emphasized across the detail row.** In large Claude and Codex
    session tables, a waiting session receives a subtle amber-tinted background across the full,
    stable row. Its Status column still contains the amber glyph and explicit status word. No badge,
    new column, or row-size change is introduced.

18. **Accounts are discovered independently of sessions.** A successfully discovered account can
    be shown with zero sessions. Loading, degraded, and error states never render as “0 accounts.”
    Canonically identical configuration roots or stable identities are deduplicated.

19. **Standard and pinned account roots are deterministic.** Zaplex documents and tests the
    supported default Claude and Codex roots. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` pin a specific
    account root. An unreadable source produces an honest scan state rather than an invented
    account or empty success.

20. **Account overrides have one owner.** Alias, color, ordering, and hidden state continue to use
    the existing account-instance override file. Navigation and presentation do not introduce a
    second settings mechanism.

21. **Current and legacy Claude live sessions are classified truthfully.** Legacy registry entries
    with a valid status and current real conversations identified as `interactive` or `bg` remain
    eligible. Shell helpers and unknown status-less entries remain excluded.

22. **Dormant Claude history remains resumable.** A recent, substantial, valid transcript stays
    discoverable in its account detail pane after its live registry row disappears. Dormant
    transcript history is never injected into the live sidebar tree and never presented as a
    running process.

23. **Fresh reference audits are part of the feature.** The baseline, post-discovery, and final-UI
    audits each start from freshly fetched default branches of Zaplex, claudeplex, and
    claudeplex-desktop. Each audit records branch, commit SHA, time, observed reference behavior,
    Zaplex behavior, classification, and reproducible evidence. An older cached checkout is not a
    valid benchmark.

24. **Reference parity is selective.** Reference behavior may be adopted, intentionally diverged
    from, or recorded as a remaining gap. The references never become runtime dependencies and
    Zaplex never shells out to them.

25. **Responsive and accessible behavior is preserved.** Normal and narrow sidebar widths keep
    labels, chevrons, glyphs, and connection controls stable without overlap. Keyboard/focus routes
    remain usable, state does not depend on color alone, and reduced-motion preferences are
    honored.

26. **The Cockpit has a stable machine-readable snapshot.** A versioned JSON command exposes the
    same local and connected-host account/session truth without ANSI presentation or secrets.
    Unknown usage remains unknown rather than becoming a numeric zero. A partial/degraded snapshot
    is returned with a distinct non-success exit status, while a fully loaded snapshot succeeds.

27. **Remote accounts use opaque daemon-owned routes.** Account inventory crosses the daemon
    boundary as opaque account id, provider, display identity, health, and capacity facts only.
    Host filesystem paths and credentials never cross that boundary. Launch, resume, fork, attach,
    and transcript reads either preserve the exact account id or stop visibly; they never guess by
    host path, email, or least-loaded account when the data is ambiguous.

28. **Launch metadata binds to the exact provider conversation.** Model and effort intent receives
    a launch id before the command starts, survives either terminal-first or hook-first ordering,
    and is promoted to the exact host/provider/account/session identity when the structured hook
    arrives. Any coordinate-only fallback is visibly marked as estimated and is bounded in memory.

29. **Claude and Codex transcripts share one safe display projection.** Local and supported remote
    history is parsed into user/assistant turns with bounded text, thinking, model, and tool names.
    Raw tool payloads/results, developer instructions, encrypted data, credentials, and source paths
    are excluded. Missing, empty, unsupported, malformed, unavailable, and oversized histories are
    distinct states; remote requests use opaque account/session ids and revision-based refreshes.

30. **Parity is continuously checked.** Pull requests that touch Cockpit, daemon routing, provider
    discovery, or transcript code run the executable reference-parity matrix. The gate validates
    fresh reference revisions, targeted checks, responsive/reduced-motion screenshots, and a
    documented real two-host smoke procedure; missing or stale evidence fails closed.
