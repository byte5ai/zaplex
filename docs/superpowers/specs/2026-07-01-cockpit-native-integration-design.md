# Cockpit — Native Integration Design (claudeplex, adapted the zaplex way)

> **Goal.** A claudeplex user must find **everything** they had — at least parity,
> ideally more — but it must feel like claudeplex was built into zaplex *from day one*,
> not bolted on. We adapt, we don't blind-port. Precedent: the file-manager
> integration ([[filemanager-pane-mode-design]]) turned claudeplex-style "two fixed
> MC panes" into a **dynamic pane-mode on zaplex's native pane mechanism**. This doc
> does the same reframing for the cockpit.
>
> Supersedes the increment plan (§8) of the Increment-1 data-spine doc
> (2026-06-30-…); the data spine + app wiring already built ([[ui-ux-backlog-progress]],
> crate `zaplex_cockpit`, `app/src/cockpit/`) slot in unchanged as the data layer.

## 1. The reframe (why this isn't a merge)

claudeplex is a **cockpit wrapped around external infrastructure**: it shells out to
the `claude` CLI, keeps agents alive in **tmux**, browses hosts over its own **SFTP
commander**, and runs a **remote-control fleet** to survive lid-close. It had to build
all that because a TUI has none of it.

**zaplex already owns every one of those primitives, natively:**

| claudeplex builds… | …because zaplex already has it natively |
|---|---|
| tmux "remote-control fleet" (persistent, survives lid, RAM ceiling) | the **remote-session daemon** — persistent sessions, reconnect-replay, per-host **ring ceiling** ([[remote-session-arch-decision]], [[stage2-trigger-decision-option-b]]) |
| its own SFTP "Multi-host Commander" + host↔host copy | the **SSH manager** + the **file-manager pane-mode** (host↔host copy is P2/P3 there) |
| shelling out to `claude` + a parallel "agent registry" | **CLI agents as native terminal panes** (`add_tab_with_specific_agent`, the "Coding Agents" menu) — local *or* on a remote host via the daemon |
| "adopt a waiting session" | the daemon **adopt-session** feature (already built) |
| TranscriptView | **ConversationListView** (local BYOP history) + native scrollback |
| Lumen themes / bilingual / command palette | zaplex themes (Zaplex Dark), i18n (`warp.ftl`), command palette |

So the cockpit is **not** the infrastructure — it is the **intelligence & orchestration
layer** that was missing: *which account has headroom, what needs my attention, what am
I spending, launch the next agent on the freest account.* Everything else is a **lens
over zaplex's existing panes / daemon sessions / hosts**, never a parallel world.

This is the anti-foreign-body principle: the cockpit's "sessions" ARE zaplex panes; its
"launch" opens a native pane; its "fleet" IS the daemon; its "commander" IS the SSH
manager + FM pane. A claudeplex refugee sees all their features — expressed in zaplex's
own vocabulary.

## 2. Parity checklist (claudeplex → native zaplex home)

Every claudeplex feature, and where it lives natively. **Nothing is dropped.**

| claudeplex feature | Native zaplex home | Status |
|---|---|---|
| Multi-account **load dashboard** (5h/wk usage, cost, reset, heat) | **Cockpit pane** (card grid over the data spine) | spine done; UI = this plan |
| Per-model heat (opus/sonnet bars) | data-spine extension: per-model window totals | small spine add |
| **Session monitor** (live/recent across accounts, grouped by folder, status active/monitor/waiting/stale, output tails) | **unified session inventory** = live zaplex panes + daemon sessions + discovered CLI transcripts, correlated to accounts | Increment C3 |
| **New-agent wizard** (account-by-capacity → folder, or folder → account) | native **"New Agent" launch** = `add_tab_with_specific_agent` + **account routing** (launch under the chosen/freest account's `CLAUDE_CONFIG_DIR`) + repo/worktree picker | Increment C4 |
| **Agent cockpit** (message, images, restart, kill) | it *is* a zaplex terminal pane running the agent — native drive; restart/kill/paste are pane actions | native today |
| **Adopt** waiting sessions | daemon **adopt-session** (built) surfaced from the cockpit | wire-up |
| Auto-discovery of accounts | data spine (`zaplex_cockpit::discover`) | done |
| **Multi-host Commander** (hosts, SFTP, host↔host copy, remote PTY) | SSH manager + **FM pane-mode** + remote terminal panes via daemon | via those tracks |
| **Remote-control fleet** (persistent, survives lid, RAM ceiling, serves mobile) | the **daemon session layer** (persistence + ring ceiling); mobile = MCP back-channel / mobile-companion roadmap | daemon done |
| Themes / bilingual / command palette / scriptable `--json` | zaplex themes, i18n, palette; `--json` = dump `CockpitSnapshot` (already serde) | mostly done |
| GitHub issue/PR/triage modals | agent-driven actions via the **Oz-repurpose "run on my agent"** pattern ([[…oz-repurpose…]]) + cockpit quick-actions | later increment |
| Account **drill-in** detail | cockpit pane detail/filter mode | Increment C2 |
| **Codex** (net-new; claudeplex is Claude-only) | first-class in the spine already → **more than claudeplex** | done |

## 3. Native surfaces

Three complementary surfaces, each on an existing zaplex mechanism — not one monolith.

### 3.1 The Cockpit pane (the dashboard) — pane-mode, like FM
The `OverviewMain` equivalent is a **pane in the main area** (the FM/SFTP pane
mechanism, `PaneContent`), openable as tab/split, promotable, multi-instance. Contents
(over the data spine): aggregate header (accounts · cost 5h·today·wk · attention count)
+ a responsive **account-card grid** (`minmax(320px,1fr)`): accent stripe, key + label +
**plan badge** + status, email, **reset timers** (5h/wk), **heat bars** (5h/wk, later
per-model), **cost + tokens** (5h·today·wk). Card → drill-in (detail mode of the same
pane). This is the roomy dashboard both references use.

### 3.2 The live inventory IS zaplex's sessions (the key adaptation)
claudeplex keeps a **parallel** agent registry. zaplex must **not**. The cockpit's
"sessions" are unified from what zaplex already knows:
- **Live panes** — terminal panes currently running a CLI agent (local or remote), with
  their real state (running/waiting-for-input/idle) from the terminal/agent model.
- **Daemon sessions** — persistent remote sessions (attached or detached) from the
  remote-session manager, across hosts.
- **Historic** — sessions discovered from CLI transcripts (`~/.claude`/`~/.codex`),
  for accounts with no live pane.
One session model, correlated to its account (by the config-dir/env it launched under)
and host. "Open" = **focus the existing pane** (or adopt a daemon session, or open its
transcript via ConversationListView) — never spawn a duplicate.

### 3.3 Ambient attention (glanceable, no pane needed)
The mission is *reduce mental load at a glance*. So a **lightweight attention indicator**
lives in always-visible chrome (tab-bar/status area): the max account heat + a
"needs-you" count (waiting sessions). Click → open the Cockpit pane. This is the
"glanceable" value without forcing the dashboard open — the ambient half of the FM-style
"you choose where it lives" philosophy.

## 4. Actions, expressed natively
- **Launch agent** → the existing CLI-agent launch, extended with **account routing**:
  "New Agent on <account>" or **"on freest"** (lowest `work`, non-busy) sets the launch
  env (`CLAUDE_CONFIG_DIR` / Codex equivalent) — never an API key. Opens a native pane
  (local or, via the daemon, on a chosen host).
- **Adopt** a waiting/detached session → daemon adopt (built).
- **Open / drive** → focus the pane; drive = the terminal itself.
- **Commander** (host↔host files, remote PTY) → SSH manager + FM pane + daemon terminal.
- **Fix/ask/GitHub** → the Oz-repurpose "run this on my agent" one-shot.

## 5. Revised increment plan (native-first)

- **C1 — Data spine.** ✅ done (`zaplex_cockpit` + `CockpitModel` + watch + settings).
- **C2 — Cockpit pane (dashboard).** Account-card grid over the spine (plan/heat/cost/
  reset), aggregate header, drill-in detail; pure formatting helpers unit-tested. + the
  `--json`/debug snapshot dump. *(This is "Increment 2 UI", now as a native pane.)*
- **C3 — Unified live inventory.** Correlate live panes + daemon sessions + transcript
  history into one account-scoped session list with status; "open/adopt/focus" wiring;
  ambient attention indicator. Adds per-model heat to the spine.
- **C4 — Orchestration.** New-Agent launch with **account routing** + **launch-on-freest**;
  repo/worktree picker; remote-host target via the daemon.
- **C5 — Multi-host aggregation + history.** Aggregate per-host snapshots over the
  daemon; optional persisted trends; GitHub quick-actions via the agent.

Each increment is a lens/action on existing primitives, shippable on its own, and keeps
the "not a foreign body" test: *would this have looked out of place if claudeplex had
been built into zaplex from the start?*

## 6. Open questions
1. **Ambient indicator placement** — tab bar vs. a status strip vs. the left toolbelt?
   (Needs a build to feel out.)
2. **Cockpit pane vs. also a compact toolbelt summary** — do we want both, or is the
   pane + ambient indicator enough?
3. **Account routing mechanics for Codex** — Claude uses `CLAUDE_CONFIG_DIR`; the Codex
   multi-account/override story is still unconfirmed (spine §10).
4. **Per-model heat** needs the spine to split window totals by model — small change,
   do it in C2/C3.

## Appendix — reference files
- claudeplex TUI dashboard: `~/projects/zaplex/claudeplex/src/render.ts` (card grid
  `boxCard`/`gridRow`, `MIN_CARD=54`), `ui.ts` (`heat()` thresholds), `usage.ts`,
  `instances.ts` (launch-on-freest), `hosts.ts`/`remote.ts` (commander/fleet).
- claudeplex-desktop: `views/OverviewMain.tsx` (the dashboard we mirror),
  `AccountsSidebar.tsx`, `AccountDetail.tsx`, `components/LoadBar.tsx` (heat colours),
  `shell/{Shell,ActivityBar,StatusBar}.tsx`, `views/{SessionsMain,NewAgentWizard,CockpitMain}.tsx`.
- zaplex natives: `crates/zaplex_cockpit` + `app/src/cockpit/` (spine),
  daemon session layer, `app/src/ssh_manager/`, the FM pane-mode design, CLI agents
  (`app/src/terminal/cli_agent.rs`), `ConversationListView`.
