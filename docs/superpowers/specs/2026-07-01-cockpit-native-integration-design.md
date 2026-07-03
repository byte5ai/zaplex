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

## 3. Native surfaces — three glance tiers (main-pane-vs-sidebar principle)

Principle (user, approved 2026-07-01): the **main-area pane** is for the *overview /
deep dive* — big, roomy, nothing cramped; the **sidebar** is for what you want in view /
one click away *while working* — because nothing in the main area gets switched or
overlaid (your terminals/agents stay put). Applied, the cockpit is three tiers, each on
an existing zaplex mechanism — not one monolith.

### 3.1 Toolbelt icon — the ambient indicator (always visible, even collapsed)
The Cockpit toolbelt icon carries an **attention badge**: colour = highest account heat,
count = "needs-you" sessions. You read the fleet's state without opening anything.

### 3.2 Cockpit sidebar (toolbelt tab) — quick-access while working
A docked toolbelt tab (like SSH/Files), one click / hotkey, **no main-area switch**:
- compact **account list** — mini heat bar / %, plan badge, live dot, "needs-you" marker;
- **live-session quick-list** (active/waiting) → click **focuses the existing pane**
  (never a duplicate);
- **quick-launch** — "New Agent" (on freest / on account) opens a pane in the main area.
The operational daily-driver. This tier + the badge ARE the "glanceable" half — there is
no separate tab-bar widget.

### 3.3 Cockpit pane (main area) — the overview / deep dive, pane-mode like FM
Opened when you want the big picture (`PaneContent`, tab/split/promotable, multi-instance):
the full responsive **account-card grid** (`minmax(320px,1fr)`) — 5h/today/week cost+tokens
matrix, all **heat bars** (5h/wk, later per-model), **reset timers**, session breakdown,
aggregate header — plus account **drill-in** and (later) history/trends. The roomy
dashboard both references use.

### 3.4 The live inventory IS zaplex's sessions (the key adaptation)
Both the sidebar's session list and the pane's session sections draw from ONE unified
inventory — never a parallel registry:
- **Live panes** — terminal panes running a CLI agent (local or remote), with real state
  (running / waiting-for-input / idle) from the terminal/agent model.
- **Daemon sessions** — persistent remote sessions (attached/detached) across hosts.
- **Historic** — sessions discovered from CLI transcripts, for accounts with no live pane.
Correlated to account (by the config-dir/env launched under) + host. "Open" = **focus the
existing pane** / adopt a daemon session / open its transcript (ConversationListView) —
never a duplicate. This is the anti-foreign-body core.

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
- **C2 — Sidebar + pane (spine-backed, read-only). Sidebar first (approved).** Both
  surfaces share the data + card/formatting components (formatting helpers headless-
  tested). Order: (a) the Cockpit **sidebar** toolbelt tab — account glance
  (plan/heat/cost/reset) + the toolbelt icon with the heat part of the attention badge;
  (b) the Cockpit **pane** — the full card grid + drill-in over the same components; +
  the `--json`/debug snapshot dump.
- **C3 — Unified live inventory.** Correlate live panes + daemon sessions + transcript
  history into one account-scoped session list with status; powers the sidebar's
  **session quick-list + "needs-you" marker**, the pane's session sections, and the
  badge's count; adds per-model heat. "open/adopt/focus" wiring.
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
