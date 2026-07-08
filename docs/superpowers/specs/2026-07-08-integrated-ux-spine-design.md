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
