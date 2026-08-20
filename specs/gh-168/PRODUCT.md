# Managed Remote-Control Fleet

## Summary

Zaplex manages durable remote AI sessions as an explicit fleet keyed by host, account, project, and agent. Managed sessions stay available after UI detach or app restart, can be started, stopped, restarted, and attached without ambiguous process matching, and are protected by a fail-closed host-memory headroom gate.

## Goals

- Reach or exceed the remote-control fleet behavior of `claudeplex` without depending on `tmux`, shell process scans, or client-side credential paths.
- Use Zaplex's daemon-owned PTYs and opaque daemon-local account routing as the durable execution boundary.
- Integrate Claude's official `remote-control` command so its supported mobile/attach flow remains available.

## Non-goals

- Reimplementing Claude's remote-control protocol, proxying its credentials, or extracting its private sharing tokens.
- Automatically killing healthy managed agents to make room for a new launch.
- Treating output-ring bytes as agent-process memory.

## Figma

Figma: none provided. The existing approved Cockpit mockups in GitHub issue #160 define the visual density: the sidebar tree remains free of memory metrics; compact fleet details belong in the main Cockpit pane.

## Behavior

1. Every managed fleet entry has one stable, exact identity: host, opaque daemon-local account id, project root on that host, provider, and daemon PTY session id plus generation. Actions never select a target by a display label, email address, process name, or filesystem path from another host.

2. Starting a managed session is available only when the connected daemon advertises the managed-fleet capability and the selected provider/account/project route is valid on that daemon. Older daemons show the action as unavailable; Zaplex does not silently fall back to an unmanaged session.

3. A managed session is owned by the daemon. Closing a Zaplex tab, disconnecting the transport, quitting Zaplex, or restarting the client detaches the UI but does not stop or age-reap the managed session. Reconnecting lists the same session id and generation and allows it to be attached again.

4. Stop, restart, and attach require the exact session id and generation returned by the daemon. A stale generation, changed account route, changed project, changed provider, or changed foreground binding fails visibly without touching the replacement session.

5. Stop terminates the selected managed PTY and its process group, waits for process exit, removes it from the daemon inventory, and reports success only after that exact generation is gone. A process that already exited produces an idempotent not-running result rather than targeting another process.

6. Restart performs a generation-checked stop followed by a new managed start with the same host, account, project, provider, and launch configuration. The replacement receives a new PTY session id/generation; partial failure is visible and never presented as a successful restart.

7. Attach uses the existing daemon replay path, including the frozen bootstrap preamble and exact foreground-agent binding. Reattach after a transport interruption cannot attach to an unrelated PTY that reused a client-side display row.

8. Before opening a managed session, the daemon measures host headroom and evaluates a daemon-side configurable minimum. If available memory is below the configured minimum, the start is blocked before process creation with the measured value, required value, and measurement provenance.

9. The managed-start headroom check fails closed when a trustworthy host measurement is unavailable. The user sees that memory is unavailable or unsupported; it is never shown as zero bytes and Zaplex does not guess that starting is safe. Existing unmanaged terminal starts retain their current behavior.

10. On Linux, host headroom is read from `MemAvailable` in `/proc/meminfo`. Values are parsed only when their unit and range are valid. The displayed provenance names this source.

11. On Linux, managed process memory is measured as proportional set size (PSS) from `/proc/<pid>/smaps_rollup`. The daemon may aggregate the exact managed process tree, deduplicating PIDs. A missing, permission-denied, malformed, or raced-away file produces an unavailable measurement for that process; it is never silently converted to zero or mixed with an unlabelled estimate.

12. On platforms without Linux procfs, host and process memory explicitly degrade to unsupported unless that platform has an equally trustworthy native implementation. Unsupported measurement is distinct from a measured zero-byte process.

13. Measurements carry provenance and collection time. A memory snapshot for a different PTY generation or a snapshot older than the daemon's freshness limit cannot authorize a new start.

14. The headroom limit is enforced by the daemon even if a client omits a requested minimum or requests a weaker one. A client may ask for a stricter minimum; it cannot lower the configured daemon floor.

15. Memory arithmetic is overflow-safe. Invalid configuration, impossible values, or incomplete measurement block managed starts with a generic diagnostic that contains no environment variables, command lines, tokens, or credential paths.

16. Claude managed starts use the official `claude remote-control` command and documented flags. Zaplex does not implement an unofficial public listener, scrape private protocol data, or transmit an Anthropic credential to the client.

17. The official Claude mobile/share experience remains inside the managed PTY. Attaching to the managed session exposes Claude's own supported pairing/link UI. Zaplex labels this action as Claude Remote Control and does not copy or log any pairing secret automatically.

18. Claude and Codex both support managed interactive sessions using their own account route and official CLI entrypoint. Claude additionally offers the distinct official Remote Control mode. Providers without a supported managed entrypoint report that capability as unsupported; Zaplex never launches one provider under another provider's account or emulates Remote Control with a generic server.

19. The daemon binds only to the existing authenticated remote-server transport. Managed-fleet support creates no TCP/HTTP listener, changes no public bind address, and does not make Claude's sharing credentials part of Zaplex protocol messages, logs, snapshots, or UI metadata.

20. The sidebar Cockpit tree keeps its existing Host > Project > Session > Agent hierarchy and status-only density. It may indicate that a session is managed with one compact icon/state, but it does not add PSS, headroom, port, or command-line columns.

21. Selecting a managed session in the main Cockpit pane shows a compact detail group: managed state, exact provider/account label, project, process memory with provenance, host available memory against the configured floor, and the available Start/Stop/Restart/Attach/Claude Mobile actions. Unknown values use an em dash plus a short degraded explanation, not `0`.

22. Loading, unsupported, blocked, disconnected, stopped, and process-ended states are visually distinct. Controls that cannot be safe in the current state are absent or disabled with a concise reason; no optimistic state hides a daemon error.

23. Concurrent clients may list or attach the same managed session, but a mutating lifecycle operation is generation checked. Two simultaneous starts for the same host/account/project/provider launch key are idempotent: one existing managed session is returned, rather than creating duplicate servers.

24. After an unexpected process end, the daemon removes the live entry, retains a bounded non-secret exit record long enough for the UI to explain what happened, and permits an explicit restart. It does not relaunch indefinitely without user intent.

25. Fleet discovery, memory collection, lifecycle errors, and sharing metadata remain secret-safe. Account ids are opaque and daemon-local, filesystem configuration paths remain host-only, and diagnostics must not include process environments, raw command lines, OAuth data, API keys, session pairing tokens, or transcript contents.
