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
It embeds the anonymized [interactive workspace reference](../../docs/ui/premium-workspace.html)
from the 2026-09-20 design iteration, versioned on 2026-09-23. Use the
[comparison guide](../../docs/ui/README.md) to compare the already integrated implementation
with that artifact; neither the mockup nor this synchronization proves native acceptance.
The previous static illustrations are superseded, not alternative approved layouts.

## Goals

- Keep connection configuration, favorites, and live session observation distinct and predictable.
- Make local and connected-remote AI work visible without mirroring offline registry hosts.
- Restore truthful Claude history and account discovery behavior.
- Minimize sidebar noise while preserving unambiguous provider, state, and accessibility semantics.
- Preserve pane-local host/session identity across mixed-host splits, mode changes, drag, reconnect,
  and restore.

## Non-goals

- No new app-wide icon rail or navigation system. Adding the „KI-Konten“ entry to the existing
  horizontal toolbelt (#504) is not a new navigation system.
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
   Connections registry. Its first level shows favorite hosts only; every other registered host
   appears only in the "More hosts" submenu (#32). Clicking the host label connects exactly
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

   **Display rule „B+“ (#505).** The tree shows this model without empty levels. A project with
   exactly one PTY session is a single row: the project name leads and keeps priority, the
   session's own title (name, branch, worktree, or distinct directory) follows dimmed after some
   air and gives way first; a title that only repeats the project name is omitted. With one agent
   that row carries the agent; with several, the agents follow it. In a project with several PTY
   sessions, a session with one agent is one row: its title, then a second line with provider
   icon, provider, and model; an untitled session there shows its agent as the one-line headline.
   Only a PTY session with several agents keeps agent rows, and its row carries no aggregate
   glyph. A managed-fleet agent (#168) ends its provider/model line with a `◆` marker whose
   localized tooltip explains it; details stay in the account pane. A title sharing a long,
   separator-bounded prefix with at least one sibling shows it shortened and dimmed while at
   least eight distinguishing characters stay visible. A long title keeps a short last segment
   and shortens in the middle (`feat/checkout-re…-step-2`); otherwise it ends in an ellipsis.
   Whenever the visible title can be shorter, the full title is the tooltip. Title parts,
   provider, and model are separated by air and tone, never by separator
   glyphs such as a middle dot. Every row reserves the chevron and glyph columns, so titles of
   one depth share a text axis. Idle session titles use the muted text role, and a merged row takes
   the tone of its session. The row of the focused pane's agent keeps a stable accent tint; a
   collapsed row that hides it carries the tint instead. Groups separate by spacing, not lines. Session rows
   with a title are two lines; this supersedes the earlier single-line session row. Reference:
   [`docs/ui/cockpit-sessions-tree.html`](../../docs/ui/cockpit-sessions-tree.html).

   The tree contains only agent sessions Zaplex can open, decided by the same reachability rule
   as a click: a Zaplex pane hosts the session, a remote live session has a reattachable
   foreground daemon PTY with a registry route, or the session is dormant and resumes safely.
   Agents started outside Zaplex that it cannot open this way are not shown anywhere — not in the
   tree, the counter, the Dock badge, the inbox, the palette, account tables, or the dashboard.
   Account usage figures (tokens, cost, limits) still include their spend, because they describe
   the account and must match the provider's own numbers. Zaplex ownership does not depend on the
   opt-in hook bridge: a local agent process that inherited a Zaplex pane's `ZAPLEX_SURFACE_ID`
   belongs to that pane, which also becomes its click target. A session Zaplex launched or hosts
   (daemon PTY binding, exact launch binding, hook-reported id, or inherited pane id) stays visible
   while it is momentarily unreachable, e.g. during a reconnect, but is not counted until a click
   can open it. A local agent whose process cannot be inspected is never hidden.

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

12. **Waiting attention is visible but restrained.** An agent is *waiting on you* when (a) an open
    question or permission prompt blocks it — regardless of its age and of whether you looked at
    it — or (b) it finished a turn you have not seen yet, like unread mail. Opening, focusing, or
    viewing its pane in the active window marks the current turn as seen; the agent's next
    finished turn counts again. An idle session whose turn you have seen never counts, however
    long it idles, and rests as the neutral idle ring. Turns that finished before the app started
    are not counted after launch. Only the amber waiting glyph pulses. Its
    1.6-second cycle combines a modest core-brightness change with an expanding ring capped at
    approximately twice the glyph footprint. Working, idle, host-connection, and other glyphs are
    static. Reduced-motion mode replaces the animation with a static amber emphasis.

13. **Aggregate attention does not add prose and is one number.** The title-bar pulse, the Dock
    badge, the tree section header, and the attention inbox show the same count: the sessions
    from #12 that Zaplex can open right now. The tree section header uses the amber waiting glyph
    plus that count when attention exists. It does not add “N waiting.” The title-bar pulse has a
    localized tooltip and accessibility announcement naming the count and the click action.

14. **Selecting an agent is exact.** Clicking a leaf attaches to or resumes that exact agent using
    the existing stable local/remote route. Clicking the title-bar pulse opens the next counted
    session; every counted session is openable, and opening it marks its finished turn as seen.
    When nothing waits, a localized notice says so. Collapsing or expanding a row never changes
    routing, account association, waiting order, guardrails, or capability gates.

15. **Account identity has one hierarchy on every surface.** Sidebar account cards and large
    account panes use the provider (`Claude` or `Codex`) as the headline. The subordinate line
    contains the account label or email plus the plan when known. A label identical to its email is
    rendered once. Provider identity is never communicated only by color or icon, and a separate
    provider strip does not repeat the same heading above the card.

16. **Account cards retain both subscription windows.** Each sidebar account card shows the
    five-hour and weekly usage meters. The large pane stacks the same two meters and shows each
    window's reset time directly under its own meter; it may add tokens and cost provenance
    without duplicating the identity heading.

17. **Sessions needing the user are emphasized across the detail row.** In large Claude and Codex
    session tables, a session waiting on you (#12) receives a subtle amber-tinted background across the full,
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
    global feature tab is added. A favorite-host click in the `+` menu still opens a new tab; a
    pane-local split, or dragging an existing tab into the visible tab, is the explicit path for
    adding a session to the current tab.

32. **One launch menu for tabs and panes.** `+` and all four split directions (Left, Right, Up,
    Down) open the same launch menu: Terminal (local), New agent… (Cockpit enabled), the favorite
    hosts, then a "More hosts" submenu holding exactly the registered hosts that are not favorites.
    Tab-only entries (tab configs, Docker sandbox, worktree config, reopen closed session) appear
    only in the `+` menu. From a split, every entry targets the captured initiating tab, pane,
    direction, and stable host reference; a later focus change cannot redirect it. Cancel creates
    nothing. A valid selection creates exactly one session at that position and focuses usable
    input only after readiness. Same-host launch may inherit its working directory; cross-host
    launch uses the destination profile and never reinterprets a foreign path. New agent… from a
    split binds account, launch intent, and prompt prefill to the new pane and refuses a
    multi-account batch with a notice. See `docs/ui/pane-launch-and-tab-join.html`.

33. **Terminal identity is short, real, and pane-local.** The automatic pane title is
    `Host · project-or-directory`, with full host/path available accessibly. Missing metadata uses
    an honest host/session fallback. Actual short-title collisions expose the full path; identical
    full identities add a stable suffix derived from the persistent session identity. Moving or
    restoring a pane does not change that suffix; non-colliding titles stay short.
    The automatic tab title follows the focused pane through the existing title-priority rules;
    an explicit title wins until removed. Account panes retain their native Cockpit identity.

34. **Pane geometry preserves session identity.** A sole pane fills the available workspace.
    Splitting affects only the chosen pane. Dragging a pane header to a target edge moves that
    existing pane left, right, above, or below without opening a connection, PTY, or agent. Moving
    to another tab remains supported. Dragging an inactive tab out of the tab bar (or the vertical
    tabs panel) onto a pane of the visible tab inserts all of its panes, with their layout, at the
    indicated edge, inside that pane's former space; other panes keep their sizes. The source tab
    closes and no connection, PTY, or agent restarts; child-agent bindings move along. Tabs with
    hidden running panes, a pending conversation restore or summarization, or a provisional daemon
    start offer no drop target, and editor panes join only a tab without editors. Tabs activate on
    click so the visible tab stays visible while another tab is dragged. Invalid or cancelled
    drops leave the layout intact, and file drag, text selection, splitter resize, and
    header-button clicks are not pane moves.

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
    exists. Closing after navigation adopts the last successfully opened directory in the owning
    shell (#469), including after layout restore. An idle shell changes directory immediately; a
    running process is left untouched and the change waits for a ready prompt. The input draft is
    retained. A newer shell command or reopening the File Manager supersedes a deferred change.
    A changed session/host or failed connection must never redirect a foreign path into another shell.

38. **The File Manager function bar always shows every key with its caption.** F2–F8 and F10 and
    their actions are visible with a short one-word caption in every File Manager pane; only the
    focused pane enables them. Unfocused panes keep the same disabled geometry. Focus in a terminal,
    account pane, or overlay prevents file actions from firing elsewhere. A pane too narrow for one
    row wraps the bar into two rows of equal cells (four in very narrow panes); the bar never
    scrolls, hides captions, or overlaps, and existing commands/shortcuts remain available.

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
    it instead of creating a duplicate. Replay text alone is not proof of readiness.
    An exactly identified existing PTY whose original shell-start metadata has been lost may finish
    in an explicitly labelled simple terminal mode. The same process and saved input draft remain;
    typing and pasting go directly to the terminal after replay completes. Integrated command input,
    completion, automatic directory changes, and agent/startup automation stay unavailable. A later
    reconnect gates manual input again without losing the mode or editing the hidden draft. New
    sessions and ambiguous identities cannot use this fallback to bypass their startup checks.
    A restored pane whose saved remote identity is corrupt remains visibly present with its siblings, names the
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
    The live tree („KI-Sessions“) and the account cards („KI-Konten“) are two separate,
    labelled entries of the existing horizontal sidebar toolbelt, each with the full sidebar
    height and its own scroll state (#504). A session inventory of any length therefore never
    pushes the accounts out of reach. While another view is active, the Sessions entry carries
    the amber waiting mark when an agent needs the user. This supersedes the earlier
    three-fifths height cap inside one Cockpit view.

## Verbindliche Bedienungs- und Refresh-Korrekturen

- Die Cockpit-Überschrift lautet „KI-Sessions“. Gezählt werden die angezeigten Agent-Container; reine Shell-Sessions liegen unter Verbindungen, erreichbar über die vorhandene Sidebar-Navigation. Kein zusätzlicher Cockpit-Link dupliziert diesen Wechsel. Exakte bekannte Modell-IDs bleiben sichtbar.
- Die Hostzeile unter Verbindungen zeigt bei resilienten Hosts dauerhaft sichtbare Aufklapp- und Refresh-Aktionen. Refresh öffnet den Recovery-Bereich bei Bedarf. Hostnamen erhalten die gesamte verbleibende Breite mit Ellipse. Verbindungs- und Refresh-Status nutzen feste Symbolplätze; laufende Aktualisierungen schieben vorhandene Sessionzeilen nicht nach unten.
- Zaplex- und tmux/byobu-Sessions haben denselben primären Klickbereich und dieselbe Öffnen-Aktion. Pfeiltasten navigieren sichtbare Zeilen; Enter öffnet/aktiviert, Links/Rechts klappen auf oder zu, Cmd/Ctrl-R aktualisiert den ausgewählten Host. Eingabefelder und geöffnete Menüs behalten ihre eigene Tastaturbedienung.
- Übernommene Sessions zeigen tatsächlichen Verbindungs-/Installationsfortschritt. Die bestehende Startfrist von 60 Sekunden bleibt erhalten und beendet keine entfernte Session.
- Lokale Cockpit-Daten erscheinen vor Remote-Abfragen. Hosts werden parallel abgefragt und einzeln veröffentlicht; jede Inventar-RPC hat eine Frist von zehn Sekunden. Ein fehlerhafter Host bleibt mit ehrlichem Fehlerstatus sichtbar und löscht nur sein eigenes veraltetes Inventar. Ergebnisse veralteter Topologien bleiben verworfen.
- Die SFTP-Prüfung bekannter Schlüssel skaliert linear mit der Datei. Gehashte Hosts, Aliaslisten, Ports und unterschiedliche Algorithmen behalten ihre bisherigen Identitätsgrenzen; bestätigte Schlüsselabweichungen werden nicht automatisch übernommen.
