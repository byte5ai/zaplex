# Integrated UX Spine — reconciliation & fix ledger

Status: **authoritative source of truth** · 2026-07-08 · consolidates and *enforces* the
already-approved concepts that the shipped build did not honour.

> This doc does not invent a new concept. It reconciles the three approved sources below,
> records where the **shipped build diverged** from them, and turns that into a prioritized,
> code-bound fix ledger. It lives in git precisely so the concept can no longer "get lost
> between planning and implementation" — the failure the user (rightly) called out.

**Authoritative sources (unchanged, still binding):**
- `memory/agent-cockpit-master-concept.md` — the Nordstern (user-approved 2026-07-06).
- `docs/superpowers/specs/2026-07-05-cockpit-c4-launcher-design.md` — the launcher (user-decided 2026-07-05).
- `docs/superpowers/specs/2026-07-01-cockpit-native-integration-design.md` — native surfaces / tiers.

## 0. The one sentence

zaplex is **one integrated tool**, not tools crammed into a window. The value is that
*everything is a lens/action on one object model* — `Host ▸ Project(Repo) ▸ Session ▸ Agent
{model·effort·context·state}` — with **one operating grammar** and **one premium visual
language**. If a feature is built but not reachable, or reachable but incoherent with the
rest, it does not count as done.

## 1. Divergences in the shipped build (evidence-backed)

| # | Approved rule (source) | Shipped reality (code) | Verdict |
|---|---|---|---|
| D1 | Titlebar = **attention pulse** "✋ N waiting"; icons that launch blindly were **de-listed** (master §8; C4 §4 "C4 does not re-enable the title-bar") | `render_cli_agent_titlebar_buttons` (`view.rs:19011`) renders sunburst/swirl that dispatch `AddSpecificAgentTab` → **blind local launch** | **regression** |
| D2 | App-level launch has **no implicit target**; MUST open explicit launcher (agent+host+**dir**+account) (C4 §3 #1) | Plain `Claude Code` menu entry + titlebar → `add_tab_with_specific_agent` → local tab, cwd of focused pane / `$HOME`, types bare `claude` | **regression** |
| D3 | Spawn card states host **and dir** explicitly ("cwd is steering", C4 §5) | `spawn_card.rs` has no directory picker; "Project" is read-only "Default directory" | **incomplete** |
| D4 | Conductor (`Host▸Project▸Session` tree) is the **spine** (master §8, native-integration §3) | Conductor is one of 8 sidebar icons (grid), buried; no object tree as the organizing view | **buried** |
| D5 | Every claudeplex feature must be **discoverable** (native-integration §1 goal) | File manager has **no** keybinding / palette / icon — only a pane overflow-menu item | **hidden** |
| D6 | "New session" menu curated, umgekehrte-Komplexität (master §5.5) | `unified_new_session_menu_items` (`view.rs:7745`) = flat wall of Terminal+Agent+per-agent+per-account+per-host+workflows+worktree | **noisy** |
| D7 | One visual language, "aesthetics is function" (master §5.5) | ⚡ host badge noisy; `connecting…` unstyled dev-art; account cards look clickable but aren't | **polish debt** |

## 2. Target: the integrated spine (decisions)

1. **One object model, one tree.** The Conductor `Host ▸ Project ▸ Session {model·effort·ctx·state}`
   is the primary navigation spine, waiting-first, collapsing under scale (master §5.5). Every
   other surface (terminal panes, FM, code-review, dashboard) is reachable *from* it or *alongside*
   it, never a parallel world.
2. **One launch grammar = the spawn card.** All app-level agent launches go through the explicit
   spawn card (agent · host · **directory** · account/⚡freest · model · effort), smart-prefilled
   → usually one confirm. Pane-level `＋` may pre-scope from visible context. No surface launches
   blindly.
3. **Titlebar = attention, not launch.** The titlebar carries the ambient attention pulse
   ("✋ N") that jumps to the next waiting agent — never Claude/Codex launch buttons.
4. **Everything discoverable.** Every built capability has a keybinding **and** a command-palette
   entry **and** a visible affordance. "Built" ⇒ "reachable in ≤2 obvious steps".
5. **One visual language.** ● working · ✋ waiting · context green→red; calm by default, detail on
   demand; premium, quiet, consistent. Aesthetics is function.
6. **Nothing ships unverified.** A boot/keymap smoke test gates CI so a "does-it-even-open"
   regression (the cmd-shift-o crash) can never reach a build again.

## 3. Fix ledger → increments

**Build A (this PR — spine + discoverability + safety; build-verifiable without a live GUI):**
- A1 **Boot/keymap smoke test** in CI + unit test that parses every registered keystroke under
  `debug_assertions` (kills D-crash class; the cmd-shift-o bug would have failed it). → D-safety
- A2 **Titlebar pulse** replaces the blind launch icons: "✋ N" → jump to next waiting; opens the
  attention inbox when idle. → D1
- A3 **Launch through the spawn card**: the primary `Claude Code`/`Codex` entries + any titlebar
  launch affordance open the spawn card (never blind); add a **directory picker** to the spawn
  card; collapse the per-host/per-account permutations out of the "+" menu into the card. → D2, D3, D6
- A4 **Discoverability**: keybindings + command-palette + visible affordances for File Manager,
  Conductor/dashboard, spawn card ("New agent…"), and jump-to-next-waiting. → D5

**Build B (fast-follow — needs the user's eyes to iterate visually):**
- B1 Conductor object-tree as the visible spine (sidebar leads with it; roomy pane tree). → D4
- B2 Premium visual pass: ⚡ badge quiet/wertig, `connecting…` → worded state + spinner,
  interactive account cards, umgekehrte-Komplexität collapsing. → D7
- B3 Cockpit correctness: real weekly OAuth usage; multi-login account discovery.

## 4. Non-negotiables carried from the sources

- No quick-wins; robust only ([[no-quick-wins-sustainable-only]]).
- Claude **and** Codex verified (master §5.6).
- Anti-foreign-body: cockpit sessions ARE zaplex panes; launch opens a native pane; the fleet IS
  the daemon (native-integration §1).

## 5. Host reachability: "register once, pick everywhere"

**Problem.** WARP had no host registry, so the only way to "remember" a host was to bake its
connection into a user-authored *launch config* — that's why WARP users hand-built their dropdown
entries. zaplex is different: an SSH host is already a **first-class registered object** (SSH-manager
sidebar) and a node in the object tree (§1). Copying WARP's per-host launch-config model on top of
that registry means the user enters a host **twice** (sidebar *and* dropdown) — double data-entry,
bad UX. Removing the old auto-permutation "wall" without a replacement made hosts *unreachable* from
"+" — a regression.

**Principle.** The registry is the **single source of truth** for hosts. Every launch surface —
the "+" menu, the spawn card, the object tree — **reads** from it. A host is *entered once* and
*picked everywhere*. Multiple access points to the same registered object are good UX; a second
data-entry is not.

**Wall vs. list — the distinction that matters.** The removed clutter was the *combinatorial
cross-product* (every agent × every account × every host) rendered as flat rows — that stays gone.
Listing the **hosts themselves** is *linear* (N hosts) and belongs in "+".

**Decisions.**
1. **"+" menu reads the registry.** A `Hosts` group lists each registered host (linear). Clicking a
   host opens a **terminal** on it (same action as clicking it in the sidebar — a plain connect to an
   explicitly chosen host is not a "blind agent launch", so §2's no-blind-launch rule is not
   violated). Launching an **agent** on a host is done via the spawn card ("Neuer Agent…"), whose
   host picker already reads the same registry — so no per-host submenu is needed and the old
   per-host launch *permutations* do not return to the menu; only the hosts themselves (a linear
   list) do. *(Built: `unified_new_session_menu_items` emits one `OpenSshTerminal` item per
   registered `NodeKind::Server`; the spawn card's host row lists the same nodes.)*
2. **Launch/tab configs are for layouts, not hosts.** A saved config captures a *layout / command
   combo*; where it targets a host it stores a **reference** to the registry node (`node_id`), never
   duplicated connection data. This requires making `LeafContents::SshServer` capturable
   (`app/src/launch_configs/launch_config.rs:162` currently returns `Err(())` → must serialize a
   host-reference pane). "Save this session as an entry…" then produces a reusable entry that points
   at the registry.
3. **Spawn card + tree "+" scope from the registry.** The spawn card's host picker (§2, item #2) and
   any pane/tree `＋` resolve their host list from the same registry; pre-scoping just pre-selects a
   node.

**Increment (Build A/B boundary).**
- A: "+" menu lists registered hosts (`⏎` terminal); spawn-card host picker reads the registry.
- B: launch-config host-*reference* capture (`launch_config.rs:162`) + "Save this session as entry…";
  tree `＋` host-scoping.

**Non-goal.** No auto-generated agent×account×host permutations, ever. Hosts are listed; combinations
are composed in the spawn card at launch time.

## 6. Codex gate round 1 → fixes (this branch)

The first Codex gate returned NOT GREEN with 6 items. What changed:

1. **File manager is per-pane, not per-tab.** `OpenLocalFileManager` no longer decides local/remote from
   the tab's `ssh_tab_nodes`; it reads the **invoking (focused) pane's own session** —
   `active_session_is_local` decides local vs. remote, and a remote pane resolves its host via
   `node_for_session` (session→node reverse-lookup), falling back to the tab node then Local. A local
   split inside an SSH tab now opens the local FM; a remote pane opens its host's FM.
   (`app/src/workspace/view.rs` `OpenLocalFileManager`, `node_for_session`.)
2. **Spawn card remote semantics made explicit (labeled restriction, per Codex's sanctioned option).**
   A native folder picker can't browse a remote filesystem, so the card states plainly that a remote
   launch runs under *that host's own CLI login (no local account routing)* and lands in the pre-scoped
   project dir (from a Conductor project node) or the host home — never a silent default. Local launches
   get the full native directory picker. (`spawn_card.rs` account/Directory rows.)
3. **`models.dev` fully removed.** The `models_dev` module is deleted; the auto-fetch (`EnsureModelsDevLoaded`)
   and "Sync from models.dev" UI are gone; the internal caps path falls back to built-in substring
   heuristics. Claude/Codex context windows remain hardcoded in `zaplex_cockpit::context_window`.
4. **Stale "Settings → AI" copy replaced** everywhere (6 user-facing strings → "Settings → Agents[/Voice]").
5. **Conductor tree + remote adopt.** The tree already leads the cockpit sidebar (above the account cards).
   **Remote sessions are now clickable** and `attach_fleet_session` performs a real **in-place remote
   adopt**: resolve the inventory host to its live SSH node, build the agent's resume command, and open a
   remote terminal that resumes the same session in its cwd under the host's own login. (Local adopt
   unchanged.) (`cockpit/panel.rs` `render_conductor_row`; `workspace/view.rs` `attach_fleet_session`.)
6. **Usage honesty.** Headline = binding window (max of 5h/week/Opus/Sonnet sublimits); Codex accounts
   remain estimate-only and are marked with the `~` provenance prefix; remote hosts show no fabricated
   quota.
7. **Verification (what actually gates).** CI's `keymap_smoke_test` job runs the `warpui_core::keymap`
   cross-platform validation tests (the cmd-shift-o gate) plus the app-crate FM-toggle + spawn-card
   confirm-payload unit tests. The window-open boot smoke test is `#[ignore]`d — it needs the full app
   singleton graph the `App::test` unit harness can't provide (shared with the other view tests) — and is
   NOT a CI gate. Broader verification (`cargo check --locked -p warp`, `cargo test -p zaplex_cockpit`) is
   run locally and reported per change, not enforced in CI.

**Deliberate residual (labeled, not hidden):** ad-hoc *free-text* remote directory entry in the spawn
card is not offered — a remote dir is scoped via the Conductor project node instead. This is the
"clearly restrict and label" path Codex sanctioned, not a silent gap.

## 7. Codex gate round 2 → fixes

Round 2 returned NOT GREEN with deeper, more precise items; each is now addressed:

1. **Remote FM cwd honored on connect.** `SftpBrowserView`'s `start_path` was clobbered by
   `connect_to_server` (`realpath(".")`/`/`); the connect path now lands at the caller-provided
   `start_path`, falling back to home/root only for the plain, unscoped "SFTP Browse". A test exercises
   the connect-time initial directory. (`sftp_manager/browser.rs`.)
2. **Dashboard pane remote rows attach too.** The roomy cockpit pane mirrors the sidebar fix: remote
   Conductor rows now dispatch `AttachFleetSession` (remote in-place adopt); the review verb stays local.
   (`cockpit/pane.rs`.)
3. **No parallel launch grammar.** The legacy in-app `Agent` (`AddAgentTab`) entry is hidden when the
   cockpit is on; the spawn card is the single app-level launch path. (`workspace/view.rs` menu.)
4. **Directory is steering.** A plain "+" spawn-card open prefills the launch directory from the active
   local pane's cwd (not an implicit home), and the card summary always shows the directory (explicit
   "default (home)" when unset). (`workspace/view.rs` `open_spawn_card`; `spawn_card.rs` summary.)
5. **Freest + labels use the binding window + provenance.** `pick_freest` / `is_over_budget` rank on the
   fullest of 5h / week / Opus / Sonnet, not 5h alone; the spawn-card account and freest labels show the
   binding-window percent with the `~` estimate marker. (`zaplex_cockpit/routing.rs`, `workspace/view.rs`.)
6. **models.dev copy removed from shipped UI.** The Providers settings description no longer promotes a
   150-provider / OpenAI-compatible / Gemini / Ollama catalog; it states Claude Code / Codex run as their
   own CLIs (no setup) and "+ Add provider" is an optional manual path. (`i18n/en/warp.ftl`.)
