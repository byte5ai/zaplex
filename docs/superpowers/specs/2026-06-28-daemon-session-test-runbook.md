# Daemon Session — First Real-Host Test Runbook

> Branch `feat/stage2-client-attach`. Bring-up of the native persistent
> remote-session layer against a real SSH host. Covers the **open** flow, the
> **drop/reconnect** survival, and the **adopt-sidebar** (list + re-attach running
> sessions). B3 UDP is out of scope. This document describes acceptance steps,
> not completed runtime evidence. Record the tested commit and artifact plus
> observed GUI, replay, and recovery results in the PR.

## Build & install (handled for the tester)

- DMG: build via `test-dmg.yml` on GitHub Actions for **aarch64** with
  `fast=false`. `dmg_tag` must equal `v` plus `VERSION`; it is a build input,
  not an instruction to create a Git tag or publish a release. Use the
  `zap-test-dmg-aarch64` workflow artifact only after the signing/notarization
  and artifact-upload steps pass. Open the DMG and drag Zaplex to Applications;
  the MacBook is the test client, never a build or signing host.
- The workflow bundles the version-matched Linux x86_64 musl daemon for offline
  installation at `~/.zaplex/remote-server/zaplex-<tag>`. Other supported targets
  use the pinned [install ladder](2026-07-02-daemon-install-ladder-design.md).
  No manual daemon pre-placement is required for the bundled target.

## Preconditions

- A saved SSH host that uses **key auth** (`AuthType::Key`, or a OneKey credential
  of kind *Key*). Password hosts intentionally fall back to the normal SSH path —
  they will **not** take the daemon path in v1.
- The key is usable **non-interactively**: loaded in an ssh-agent, or unencrypted.
  (The ControlMaster spawns with `BatchMode=yes`; an encrypted key with no agent
  will fail — that's expected v1.)
- Host reachable over SSH. The remote-server binary does **not** need to be
  preinstalled — it auto-installs on first connect (watch for the install log).
- In the SSH server form, set **Session persistence = Persistent** and Save.

## Steps

1. Install and launch the signed/notarized CI artifact described above.
2. Open the saved host (the same action as a normal SSH connect).
3. A new tab should appear and, after the connect sequence, show a working remote
   shell **with Zaplex blocks/prompt** (not a bare VT).
4. Type a few commands; resize the window; try the mouse in a TUI (e.g. `htop`).
5. **Drop test:** kill the network / sleep the laptop / `pkill -f "ssh .*ControlPath"`
   briefly, then restore. The session should reconnect and replay — the shell
   state (your scrollback/running program) survives. After a *long* drop where the
   daemon ring evicted old output, the screen resets and shows a one-line notice
   `[zaplex] scrollback truncated during a long disconnect` (instead of a garbled
   grid) — that's expected.
6. **Adopt-sidebar:** expand the connected persistent host's automatically revealed
   **Zaplex sessions** section (or use the host menu → **Zaplex sessions**). It should list
   the daemon session(s) on that host (title = agent provider plus task name, falling back to project where available,
   otherwise a neutral Zaplex session identifier; never a shell executable name). Click one → it re-attaches
   in a new tab (replay + live). Adopting a session that's already open should
   **focus the existing tab**, not open a duplicate. (Only offered for key-auth /
   key-backed-OneKey hosts; otherwise you get a clear "needs key-based
   authentication" line, not a cryptic ssh error.)
7. **Add-host UX:** the saved list shows **only hosts you added** — no auto-imported
   entries. Click **+** → the "Add a host" block offers *Create a blank server* plus
   on-demand `~/.ssh/config` suggestions (the list is otherwise untouched).
8. **Failure visibility:** a failed connect/open/attach shows a red
   `[zaplex] …` notice in the tab (e.g. `connection failed (…)`, `could not start
   session: …`, `session ended`) instead of a blank/hung tab.
9. **Cross-version recovery:** with a session still owned by an older release,
   connect the current client and confirm that the old session remains listed.
   Attach and reconnect must reach that exact PTY/runtime; new work uses the
   current daemon. Historical managed sessions retain Attach/Stop, while
   Start/Restart are unavailable. Two connected runtimes may have separate
   Cockpit roots; the combined Connections list must hide their shared host-cap
   row. A daemon lacking agent inventory uses a neutral session title.
10. **Refresh:** open and end a session with the disclosure expanded, including
    while a list request is pending; the final list must reflect the latest
    lifecycle event. Collapse the section, reconnect, and confirm it stays
    collapsed until explicitly reopened.

## Expected client-log sequence (happy path)

Filter the app log for `daemon connect` and `daemon_tty:`. In order:

```
daemon connect [HOST]: establishing ControlMaster
daemon connect [HOST]: checking remote-server binary
daemon connect [HOST]: binary present            # or: binary missing — installing → install complete
daemon connect [HOST]: transport ready — connecting session SessionId(...)
daemon_tty: issuing OpenSession (cwd=…, shell=…, RxC)
daemon_tty: session opened, pty_session_id=<uuid>
```
After that, output streams into the tab. On a reconnect you'll also see:
```
daemon_tty: re-attaching pty_session_id=<uuid> from seq <n>
```

## Failure modes → meaning → likely fix

| Symptom / log | Meaning | Likely fix |
|---|---|---|
| Nothing daemon-related; opens a normal `ssh` tab | Host isn't taking the daemon path | session_resilience not `Persistent`, or auth isn't key (password/encrypted-key-without-agent) |
| `ControlMaster setup failed: …` | `ssh -f -N` couldn't authenticate/connect | key not in agent / wrong key_path / host unreachable / BatchMode rejected |
| `ControlMaster socket did not appear` | master backgrounded but no socket | `~/.ssh` not writable, or `-f` returned before socket bind — capture the ssh stderr |
| `remote-server binary check failed` / `install failed` | check/install over the master failed | host unsupported (libc/arch), or the master died between steps |
| `transport ready` but no `daemon_tty: session opened` | connect_session handshake or OpenSession failed | check the **remote** daemon log on the host (`~/.zaplex/remote-server/*/…` stderr); likely proxy/daemon spawn or protocol issue |
| Tab shows raw shell, **no blocks** | bootstrap didn't run | shell not bash/zsh/fish, or the init script didn't execute over the PTY — capture the first ~2 KB of session output |
| Reconnect doesn't replay | re-attach gap | capture `daemon_tty: re-attaching … from seq N` + whether output resumes |

## Remote-side log (on the host)

The daemon logs to its stderr (captured via the proxy). Useful lines:
`Daemon: opened session <id> …`, `Daemon: bootstrapped session <id> (bash)`,
`Daemon: attached conn … (replay … bytes …)`, `Daemon GC: reaped …`.

## What to send back

For any failure, the **last ~30 client-log lines** containing `daemon connect` /
`daemon_tty:` + what you saw on screen. If it gets to `session opened` but renders
wrong, also the first chunk of the tab's output. That's enough for me to pinpoint +
fix without a host.
