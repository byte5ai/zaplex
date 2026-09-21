# Cockpit, Connections, and account identity

GitHub: https://github.com/byte5ai/zaplex/issues/160

## Summary

Zaplex separates configured host connections from the live AI-session Cockpit while keeping both
surfaces on one stable host identity model. The Cockpit becomes a quiet live
`Host → Project → PTY session → Agent` tree, and the large Claude and Codex panes show provider,
account, usage, and attention without duplicated identity or redundant status text. The later UI
implementation contract in #459–#464 extends this surface into the existing tab/pane workspace; it
does not introduce a new navigation model.

## Figma

Figma: none provided. The binding visual reference is
[`docs/ui/cockpit-sidebar-connections.html`](../../docs/ui/cockpit-sidebar-connections.html).

## Goals

- Keep connection configuration, favorites, and live session observation distinct and predictable.
- Make local and connected-remote AI work visible without mirroring offline registry hosts.
- Restore truthful Claude history and account discovery behavior.
- Minimize sidebar noise while preserving unambiguous provider, state, and accessibility semantics.
- Preserve pane-local host/session identity across mixed-host splits, mode changes, drag, reconnect,
  and restore.

## Non-goals

- No new app-wide icon rail or navigation system.
- No provider level between host and project.
- No second connection registry or account-override store.
- No runtime dependency on either reference repository.
- No offline registry hosts in the live Cockpit tree.
- No global File Manager, Tests, host-group, or provider navigation tabs.
- No assumption that all panes in a tab belong to the same host.

## Behavior

1. **Connections is an independent sidebar element.** It lists every registered SSH host and owns
   adding, editing, deleting, connecting, disconnecting, and favoriting hosts. The Cockpit never
   duplicates those configuration controls.

2. **The Connections list is quiet by default.** Host labels use the same neutral treatment. A
   trailing connection control communicates state without prose: a connected host uses an intact
   green plug and a disconnected host uses a muted slashed plug. The list does not repeat labels
   such as “local,” “not connected,” or “2 open,” and it has no explanatory legend or live-root
   summary.

   A resilience-enabled host may expose one subordinate “Zaplex sessions” recovery disclosure.
   It is session recovery, not a second registry or a live-root summary: opening it inventories
   daemon-owned PTYs, and the first resilient connection may reveal it once so sessions surviving
   an app restart are not undiscoverable. Its rows use agent/project identity when trustworthy and
   a neutral Zaplex-session identifier otherwise; raw shell executable names are never identities.
   Buffer or runtime diagnostics are absent from the ordinary session list and remain available in
   explicit connection details under their measured name; an output buffer is never labelled as
   total host RAM.

3. **Favorites project into the tab `+` menu.** The menu reads stable host references from the
   Connections registry and shows favorite hosts only. Clicking the host label connects exactly
   once in a new tab. A separate stable `⋯` action opens a real side flyout while the parent menu
   stays visible; the flyout contains New Agent, Edit Connection, and Remove from Favorites. It
   never expands actions between rows, and clicking `⋯` never starts a connection. Editing a
   favorite never creates a second connection record. This rule supersedes the earlier
   submenu-first behavior.

4. **The Cockpit tree hierarchy is `Host → Project → PTY session → Agent`.** A project represents
   the detected repository/worktree grouping. A PTY session is the Zaplex terminal-session
   container when that identity exists. Each agent leaf represents one Claude, Codex, or other
   supported agent conversation. Agent conversations without PTY metadata receive separate stable
   fallback session containers and are never merged by label alone.

5. **Local remains visible.** The local host root and its discovered live sessions are always part
   of the Cockpit. With no local agent sessions, the root remains and shows an honest empty state.

6. **Remote roots follow actual open Zaplex sessions.** Opening the first session to a remote
   daemon adds exactly one root keyed by its stable daemon identity. Further sessions to that
   daemon reuse it. Closing or
   disconnecting the final open session removes the root synchronously without deleting the host
   or changing its favorite state in Connections.

   During explicit cross-version recovery, the still-owning historical daemon keeps a separate
   stable daemon identity. The Cockpit may therefore temporarily retain one root per connected
   daemon runtime for the same registered host; each root follows that runtime's connection
   lifecycle, including the final disconnect. It must never merge their routing identities or
   route an old session through the current daemon. Runtime-local diagnostics remain attributable
   to the exact runtime and are shown only in explicit connection details, never as an aggregate
   host-cap row in ordinary navigation.

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

31. **A tab is a pane container, not a host.** Terminal panes for local and different remote hosts
    may coexist with File Manager and account-detail panes in one tab. No automatic host group or
    global feature tab is added. A favorite-host click still opens a new tab; a pane-local split is
    the explicit path for adding a session to the current tab.

32. **Pane-local split launch is explicit.** The initiating pane offers Right and Down followed by
    the existing registered hosts and Local. The captured target is the initiating tab, pane,
    direction, and stable host reference; a later focus change cannot redirect it. Cancel creates
    nothing. A valid selection creates exactly one session at that position and focuses usable
    input only after readiness. Same-host launch may inherit its working directory; cross-host
    launch uses the destination profile and never reinterprets a foreign path.

33. **Terminal identity is short, real, and pane-local.** The automatic pane title is
    `Host · project-or-directory`, with full host/path available accessibly. Missing metadata uses
    an honest host/session fallback, and only actual collisions add a restrained disambiguator.
    The automatic tab title follows the focused pane through the existing title-priority rules;
    an explicit title wins until removed. Account panes retain their native Cockpit identity.

34. **Pane geometry preserves session identity.** A sole pane fills the available workspace.
    Splitting affects only the chosen pane. Dragging a pane header to a target edge moves that
    existing pane left, right, above, or below without opening a connection, PTY, or agent. Moving
    to another tab remains supported. Invalid or cancelled drops leave the layout intact, and
    file drag, text selection, splitter resize, and header-button clicks are not pane moves.

35. **Focus and restore are exact.** Each tab retains its last valid focused pane. Closing, moving,
    reconnecting, and restoring preserve layout, host/daemon/PTY/generation identity, working
    directory, input draft, File Manager mode, account-pane identity, and applicable selection.
    Delayed callbacks cannot steal focus back to a stale pane. A whole tab remains in its current
    window while a contained provisional daemon start still owns workspace-local routes or
    callbacks. Once a daemon pane has reached authoritative input readiness, a later transport
    reconnect gates that pane's input without reclassifying it as a provisional start. Input
    readiness alone does not release a managed start; its final managed acknowledgement or terminal
    failure does.

36. **An account click adds or focuses a detail pane.** It never replaces an existing terminal or
    File Manager pane. The same account detail is not duplicated unnecessarily within one tab, but
    may exist independently in different tabs. “All accounts” is text-labelled, not an unexplained
    directional icon.

37. **File Manager is a reversible terminal-pane mode.** Switching Terminal ↔ File Manager keeps
    the session, host, working directory, input draft, and process. F10 or mode close returns to the
    terminal rather than closing the session. Two File Manager panes form the familiar MC workflow,
    but any number of panes and destinations in other tabs remain valid; no global File Manager tab
    exists.

38. **The File Manager function bar is one stable row per pane.** F3/F4/F5/F6 and their actions are
    visible in every File Manager pane; only the focused pane enables them. Unfocused panes keep the
    same disabled geometry. Focus in a terminal, account pane, or overlay prevents file actions from
    firing elsewhere. Narrow widths and long translations do not wrap or overlap the command bar,
    and existing commands/shortcuts remain available.

39. **Transfers bind exact source and destination identity.** Copy and Move show source and target.
    A single valid visible counterpart may be the default; ambiguity requires selection, including
    File Manager panes in other tabs. Identity includes host plus stable pane/session reference and
    path, so equal paths on different hosts are distinct. Source, target, generation, mode, path,
    and transport are revalidated before execution. Existing conflict, symlink, streaming, cancel,
    and overwrite protections remain unchanged.

40. **Parent navigation restores semantic selection.** After a successful `..`, the directory just
    left is selected by identity and scrolled into view, including local/remote, sorted/filtered,
    and delayed loads. Root, cancellation, failure, removal, or invisibility uses a safe predictable
    fallback and never mutates another pane or transfer target.

41. **Start and reconnect states tell the truth.** Ordinary terminal command input is visibly gated
    until the selected PTY generation is attached, replay is handled, and input is actually usable;
    typed or pasted commands are not invisibly queued. Required authentication and host-key flows
    retain their explicit secure input. Transport, attach, replay, and ready phases are distinguished
    where known. Every start ends in ready, a concrete retryable/cancellable error, or cancellation;
    cancellation never terminates the remote work. Reopening the same already-visible session focuses
    it instead of creating a duplicate. Replay text alone is not proof of readiness. A restored pane
    whose saved remote identity is corrupt remains visibly present with its siblings, names the
    damaged restore honestly, exposes no actions that cannot work without that identity, and never
    falls back to a local shell on any platform. Retrying or cancelling a valid daemon restore also
    retains a daemon-backed fail-closed surface on every platform and never starts a local process.
    Managed opens that outlive both acknowledgement windows are cancelled before they can create an
    unowned agent or PTY. Every managed launch, including the first acknowledgement for an existing
    PTY, must claim the exact generation and verify the foreground agent in an authoritative attach
    before reporting success. Managed bulk targets for multiple accounts on one registered host each
    retain an independent connection attempt and all advance to an acknowledgement or visible
    failure; one target cannot consume the others. A late
    duplicate-owner collision discards the new surface and connection, focuses the existing owner,
    and neither persists a second identity nor transfers PTY ownership. Managed launch is unavailable
    until both client and daemon advertise the compatible attempt/attach protocol; older mixed
    versions fail closed and require an upgrade.

42. **The shared UI uses one native visual language.** Cockpit and Connections use the same section,
    hierarchy, row, indentation, and action rules while remaining a continuous sidebar surface that
    is visually distinct from work panes. Selection, focus, and action emphasis use existing theme
    roles only; no hard-coded colors or decorative pane borders are introduced. Both usage windows
    are stacked across the account-card width, and large account panes preserve existing session
    actions and cost/token provenance without repeating provider identity.

## Verbindliche Bedienungs- und Refresh-Korrekturen

- Die Cockpit-Überschrift lautet „KI-Sessions“. Gezählt werden die angezeigten Agent-Container; reine Shell-Sessions liegen unter Verbindungen und sind von hier aus direkt erreichbar. Exakte bekannte Modell-IDs bleiben sichtbar.
- Die Hostzeile unter Verbindungen zeigt bei resilienten Hosts dauerhaft sichtbare Aufklapp- und Refresh-Aktionen. Refresh öffnet den Recovery-Bereich bei Bedarf. Hostnamen erhalten die gesamte verbleibende Breite mit Ellipse. Verbindungs- und Refresh-Status nutzen feste Symbolplätze; laufende Aktualisierungen schieben vorhandene Sessionzeilen nicht nach unten.
- Zaplex- und tmux/byobu-Sessions haben denselben primären Klickbereich und dieselbe Öffnen-Aktion. Pfeiltasten navigieren sichtbare Zeilen; Enter öffnet/aktiviert, Links/Rechts klappen auf oder zu, Cmd/Ctrl-R aktualisiert den ausgewählten Host. Eingabefelder und geöffnete Menüs behalten ihre eigene Tastaturbedienung.
- Übernommene Sessions zeigen tatsächlichen Verbindungs-/Installationsfortschritt. Die bestehende Startfrist von 60 Sekunden bleibt erhalten und beendet keine entfernte Session.
- Lokale Cockpit-Daten erscheinen vor Remote-Abfragen. Hosts werden parallel abgefragt und einzeln veröffentlicht; jede Inventar-RPC hat eine Frist von zehn Sekunden. Ein fehlerhafter Host bleibt mit ehrlichem Fehlerstatus sichtbar und löscht nur sein eigenes veraltetes Inventar. Ergebnisse veralteter Topologien bleiben verworfen.
- Die SFTP-Prüfung bekannter Schlüssel skaliert linear mit der Datei. Gehashte Hosts, Aliaslisten, Ports und unterschiedliche Algorithmen behalten ihre bisherigen Identitätsgrenzen; bestätigte Schlüsselabweichungen werden nicht automatisch übernommen.
