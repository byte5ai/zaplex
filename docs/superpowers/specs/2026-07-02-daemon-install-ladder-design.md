# Daemon install ladder — design (auto-install on first connect, host-internet-optional)

> First connect to a resilient host must **auto-install the zaplex remote-server
> daemon** (like Warp) so the persistent session "just works" — but the **remote host
> must never be required to reach the internet** (remote-dev hosts are often
> locked-down / air-gapped), it must work for **any host OS/arch**, and it must show
> **progress**, degrading to a plain SSH session (with a visible banner) only if
> everything fails. Approved 2026-07-02. Supersedes the "client-push only" direction
> in [[daemon-connect-fallback]] (which itself corrected an over-eager fail-fast).

## The ladder (fast-when-possible, robust-when-not, safe-always)

On connect, if `check_binary` says the daemon is absent, install it via this ladder;
each rung is tried only if the previous is unavailable. Progress is reported to the
tab at every rung.

1. **Reachability probe (~3 s).** Host runs `curl -fsI --max-time 3 <release-url>`
   over the existing ControlMaster. Cheap; decides rung 2 vs 3 without paying a full
   download timeout on locked-down hosts.
2. **Host-side download** (probe OK). Host `curl`s the version-matched, host-platform
   tarball straight from the release + runs the install script. Fastest (host's own
   pipe, no client bandwidth).
3. **Client relay** (host unreachable, or rung 2 failed). The **client** supplies the
   host-matching binary and scp's it over the ControlMaster — host needs zero internet:
   - **3a. Embedded** binary (bundled in the .app for the common platforms) → instant
     scp, fully offline.
   - **3b. Client download** (platform not embedded / stale) → client `curl`s the
     version-matched tarball locally, then scp. Covers the open-ended platform matrix.
4. **Classic SSH + banner.** All rungs failed → open a plain local-PTY `ssh` session
   and show a prominent warning (persistent toast/banner): *no persistent session,
   a disconnect loses open work.* Never a silent hang.

**Version-match invariant:** every rung fetches the tarball for the **client's exact
version** (`GIT_RELEASE_TAG`), never "latest" — no client/daemon protocol skew (the
root of the tag-mismatch hang).

## Progress (all rungs)

A `DaemonInstallPhase` reported from the off-thread install to the daemon tab UI:
`Probing → Downloading(rung) → Uploading → Configuring → Verifying → Ready|Failed`.
The daemon tab renders a labelled bar/line ("Setting up remote session on <host> —
uploading… ▓▓▓░░"). Mechanism: an `async_channel` (or model event) the install task
sends phases on; the daemon-tty event loop consumes them and `write_notice`/renders.
Tab must exist during install to show this → create the daemon tab first (showing the
installer), and on rung-4 fallback, replace it with the classic session.

## Embed (rung 3a)

- **What:** the Linux musl server binaries for **x86_64 + arm64** (the overwhelming
  host majority), shipped as compressed tarballs in the .app **resources** (NOT
  `include_bytes!` — keep the executable lean; load from the bundle at runtime).
- **Size:** ~50–80 MB compressed each → ~+100–160 MB DMG. Acceptable for the offline
  guarantee + version-match. Other platforms fall to rung 3b (client download).
- **Code:** a resolver that returns the embedded tarball path for `(os,arch)` if
  present (reuse `detect_remote_platform`), else `None` → rung 3b.
- **CI/packaging:** `build-remote-server.yml` already builds the musl binaries; the DMG
  build (`test-dmg.yml` / release) must fetch/stage them into the bundle's resources.
  This is the one piece that needs the pipeline (can't be validated purely locally).

## Reuse (pieces already exist)
- Rung 2 host-download = `run_install_script(&socket, None, …)` (currently retired as
  primary — re-activate behind the probe).
- Rung 3b client-relay-download = today's `scp_install_fallback` (detect platform →
  client curl → scp → install script).
- `detect_remote_platform`, `download_tarball_url(platform)`, `verify_installed_binary`,
  `run_install_script(Some(staged))` — all reused.
- Classic fallback + warning toast = already built ([[daemon-connect-fallback]]).

## Implementation order (one testable build)
1. **Ladder + probe** in `install_binary` (+ a `probe_host_internet` helper): rung 1→2→3b.
2. **Embed resolver** (rung 3a) + wire ahead of 3b.
3. **Progress** phases + tab rendering (create daemon tab first; installer view).
4. **Classic+banner** on total failure (reuse existing fallback).
5. **CI/packaging**: stage the musl tarballs into the .app; then a build.

Test in one DMG: connected host → rung 2; locked-down host with embedded platform →
rung 3a (offline); exotic platform → rung 3b; no source → rung 4 (classic + banner) —
all with a progress indicator, none hanging.

## Open questions
1. Probe URL: HEAD the exact tarball asset vs. a lightweight reachability endpoint.
2. Embed compression/format (reuse the release tarball format for one code path).
3. Where the daemon tab's installer view lives (reuse the daemon-tty "starting" view +
   a phase line, vs. a small dedicated installer element).
