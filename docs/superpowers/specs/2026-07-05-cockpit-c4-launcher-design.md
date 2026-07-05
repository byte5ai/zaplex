# C4 — The Launcher / Plexing Engine (increment design)

Status: **design for approval** · 2026-07-05 · supersedes the C4 bullets in
`2026-07-01-cockpit-native-integration-design.md` (§4/§5) with a concrete increment split.

## 1. Why this is the core, not a backlog item

zaPLEX = the "plex" of claude**plex**. **Plexing = multiplexing agent work across the
user's pool of *subscriptions*** (several Claude Max/Pro logins + Codex), so no single
subscription runs dry / into rate limits while others sit idle.

That splits in two:
- **Read side — "the gauge"**: see how full each subscription is. **Already built**
  (C3a session monitoring + C3b per-account OAuth usage/quota).
- **Act side — "the gearbox"**: route/launch the next agent onto the *right* (freest)
  subscription, on the chosen host, in the chosen dir. **This is C4, and it is missing.**

Without C4 zaplex has the gauge but not the gearbox: the usage display we already
built only becomes *useful* once C4 acts on it. C4 also delivers ~80 % of Fork-remote
(#6), since a fork is "launch an agent+conversation, routed to an account/host."

## 2. What already exists (do NOT rebuild)

| Piece | Where | State |
|---|---|---|
| Account roster (per subscription) | `crates/zaplex_cockpit` `Account { provider, key, config_dir, label, plan_tier, is_default }` (`types.rs:32`); `claude::discover_accounts` (`claude.rs:127`) scans `~/.claude`, `~/.claude-*`, `$CLAUDE_CONFIG_DIR` | ✅ (Claude multi; Codex single-account, `codex.rs`) |
| Per-account capacity | `AccountUsage.heat` (5h work/budget), `provenance: Real\|Estimate` (`types.rs:158`); real numbers fetched **per config-dir** in `app/src/cockpit/oauth.rs:103` (15-min cache) | ✅ Claude; Codex = estimate only |
| Config-dir pinning primitive | `CLIAgent::fork_command_pinned(session_id, config_dir)` inline-env `CLAUDE_CONFIG_DIR=… claude …` / `CODEX_HOME=… codex …` (`cli_agent.rs:210`) | ✅ but fork-only |
| Launch-in-dir-with-command template | `fork_agent_session_in_place` (`view.rs:4160`): `NewTerminalOptions::with_initial_directory(cwd)` + `execute_command_or_set_pending(cmd)` | ✅ (the template to generalize) |
| Launch action already carrying a pin | `WorkspaceAction::ForkAgentSession { agent, cwd, config_dir, into_worktree }` (`action.rs:638`) | ✅ (shape to mirror) |
| Cockpit model + UI mounts | `CockpitModel` singleton (`model.rs:39`); per-account cards in `CockpitPaneView` already dispatch pinned fork actions (`pane.rs:131`); new-session menu lists agents as `AddSpecificAgentTab` (`view.rs:7080`) | ✅ (mount points exist) |
| Host model + connect | `SshServerInfo` (`warp_ssh_manager/types.rs:145`); `try_open_daemon_ssh_terminal` + `OpenSessionParams.startup_command` (`view.rs:6188`) | ✅ |

**Confirmed absent (the actual C4 work):** any "freest"/routing/account-selection
function (nowhere in `app/src` or the spine); a *general* pinned launch (only fork is
pinned today); the launcher UI; a per-launch **remote** cwd/command parameter.

## 3. Load-bearing decisions

1. **App-level launch has NO implicit target** (user-decided, memory `cockpit-native-integration`
   #C4-req 2). With many parallel sessions, "active session" is undefined. So the app-level
   entry MUST open an explicit launcher (agent + host + dir + account), prefilled + recents.
   Pane-level affordances MAY use visible context (that stays as-is).
2. **Route by pinning the config dir, never an API key.** Pinning = set `CLAUDE_CONFIG_DIR`
   (Claude) / `CODEX_HOME` (Codex) inline, exactly like `fork_command_pinned`. Any inherited
   API-key env must be scrubbed on the launch (belt-and-suspenders; the fork path is already
   config-dir-only).
3. **Freest = lowest 5h heat among usable, non-full accounts.** "Full" = heat ≥ a ceiling
   (align to the heat thresholds: ≥1.0 red / ≥0.85). Prefer `provenance == Real`; among
   Estimate-only, fall back to the plan-budget heat. Tie-break: higher plan tier, then recency.
   **User-decided 2026-07-05:** whether the launcher *auto-applies* freest or *shows the ranked
   list* for manual pick is a **setting** (`CockpitSettings.launch_routing = auto_freest |
   show_ranked`). `pick_freest` is built either way; the setting only picks the UI behaviour.
4. **Claude vs Codex.** Claude has real per-account usage → launch-on-freest is meaningful.
   Codex is single-account + estimate-only today → Codex "routing" is just "launch Codex"
   (no freest choice). Design the API provider-generic; light up Codex-freest when/if Codex
   grows multi-account + a usage signal. Not a blocker.
5. **cwd is steering, not detail** (C4-req 1): cwd governs CLAUDE.md discovery, so the launcher
   always states host + dir explicitly.

## 4. Increment plan (each a shippable, verified PR)

### C4-1 — Routing brain (headless, pure). *Low risk, no UI.*
- In `crates/zaplex_cockpit`: `pick_freest(provider, &[AccountUsage], now) -> Option<&Account>`
  implementing decision #3, plus a thin `routable_accounts(provider)` accessor over the
  snapshot roster. Mirrors claudeplex `instances.ts`.
- **Deliverable:** the "which subscription has room" brain, fully unit-tested (Real-wins,
  full-skipped, estimate-fallback, all-full → None, tie-breaks). No app changes.
- **Anchors:** `CockpitSnapshot.accounts` (`types.rs:189`), `AccountUsage.heat/provenance`
  (`types.rs:171,184`), `windows.rs` heat derivation.

### C4-2 — Config-dir-pinned agent launch (mechanism, local host). *Medium.*
- `CLIAgent::launch_command_pinned(config_dir: Option<&Path>) -> String` — the general
  sibling of `fork_command_pinned` (bare `claude`/`codex` + inline env, no `--resume`).
- `WorkspaceAction::LaunchAgent { agent, config_dir: Option<PathBuf>, cwd: Option<PathBuf> }`
  + handler modeled on `fork_agent_session_in_place` (`view.rs:4160`):
  `with_initial_directory(cwd)` + `execute_command_or_set_pending(launch_command_pinned)`,
  with API-key scrub. `config_dir = None` → default account.
- **Deliverable:** a routed local launch reachable via a temporary debug action/test;
  command-construction unit-tested (pin present/absent, both providers).
- **Anchors:** `cli_agent.rs:210`, `view.rs:4061/4160`, `action.rs:638`, `pane_group/mod.rs:760`.

### C4-3 — The launcher UI + freest wiring. *Medium.*
- Explicit launcher: **agent ▾ · account ▾ (incl. "⚡ freest" → `pick_freest`) · dir ▾**
  (prefilled: default account, cwd of the focused pane / last-used; folder recents per account).
  Dispatches `LaunchAgent`. Honours the `launch_routing` setting (auto_freest vs show_ranked).
- **Canonical entry (user-approved 2026-07-05): the new-session dropdown** — a "New agent
  (routed)…" entry beside `AddSpecificAgentTab` (`view.rs:7080`). Secondary: a "＋ agent"
  affordance on the cockpit per-account card (precedent `pane.rs:131`) that launches on *that*
  account. There must be a **deliberate, discoverable way to open the launcher** — the dropdown
  entry is it.
- **Title-bar icons: NOT touched (user-decided 2026-07-05, explicitly deferred).** They were
  de-listed because they launched blindly (no host/dir/account). Re-surfacing them — even as a
  "opens the launcher" shortcut — is a *separate* future UX call and requires an explicit OK.
  C4 does **not** re-enable or modify the title-bar.
- **Deliverable:** the visible C4 feature, local host. Formatting/pick logic headless-tested;
  UI wiring behind the cockpit feature flag.
- **Open UI question:** launcher as a dropdown/menu vs a small modal (recents + freest badge
  argue for a compact modal). Resolve at build (feel it out), default = menu-with-sidecar
  since that surface already exists (`configure_action_sidecar_for_hovered_item`, `view.rs:9542`).

### C4-4 — Remote-host target (ties to Fork-remote #6). *Higher, own increment.*
- Add **host ▾** to the launcher; a remote launch threads `cd <dir> && <pinned agent cmd>`
  through `OpenSessionParams.startup_command` (daemon path, `view.rs:6270`) — the one place a
  per-launch remote cwd+command can be expressed today (no per-launch remote param otherwise).
- This is where C4 meets #6: same host+dir+account plumbing; #6 adds the conversation-id.
- **Deliverable:** launch on a chosen daemon host; classic-SSH host via the startup-command
  injector as a follow-up.

## 5. Order & rationale

C4-1 → C4-2 are pure logic + mechanism (headless-testable, low blast radius) and unlock the
value even behind a debug trigger. C4-3 makes it visible and flips the title-bar icons back on.
C4-4 is remote and naturally shares scope with Fork-remote (#6), so it comes last / together.

## 6. Open questions for the user

1. **Freest ceiling + tie-break** exactly as decision #3, or do you want a different rule
   (e.g. never auto-pick, always show the ranked list)?
2. **Launcher surface** — is the new-session dropdown the right primary home, or do you want
   the cockpit pane's per-account "＋" to be the primary and the dropdown secondary?
3. **Codex** — confirm: single-account "just launch Codex" for now, revisit on multi-account.
4. **Title-bar icons** — on re-enable, should they default to "⚡ freest + last dir" one-click,
   or always open the launcher for confirmation?

## Appendix — reference

claudeplex `instances.ts` (launch-on-freest), `render.ts`/`ui.ts` (heat thresholds);
zaplex spine `crates/zaplex_cockpit`, `app/src/cockpit/`, `cli_agent.rs`, `view.rs` fork path.
Prior C4 intent: `2026-07-01-cockpit-native-integration-design.md:51,102-105,124`;
`2026-07-03-claudeplex-gap-analysis.md` §(f).
